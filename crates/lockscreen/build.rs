//! Builds the Windows resource: icon, manifest and a `VERSIONINFO` block whose
//! version comes from `Cargo.toml`.
//!
//! Follows `flex-taskbar`'s pattern, including the reason the script is
//! generated rather than kept beside the other assets: a hand-written
//! `VERSIONINFO` would be a second place to edit at release time, and the first
//! one to be forgotten.

use std::io::Write;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/app.ico");
    // The version block is built from the package version, so a release that
    // changes nothing but Cargo.toml still has to rebuild the resource. Watch
    // the file itself: `rerun-if-env-changed=CARGO_PKG_VERSION` is not enough
    // on its own, and a stale resource would claim the wrong version.
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    // Where the built copy looks for updates is baked in by `win::update` from
    // this variable. Declared here as well as read there because the CI runner
    // shares one target directory between every build, and a release for one
    // forge must not reuse an object file compiled for the other. rustc records
    // `option_env!` reads of its own, so this is belt and braces -- but a copy
    // that updates from the wrong forge is the kind of failure that is found by
    // a user, months later, and not by a test.
    println!("cargo:rerun-if-env-changed=LOCKSCREEN_RELEASES_API");

    emit_build_provenance();

    // Only Windows builds get the icon and manifest. The manifest is not
    // optional: it opts into Common Controls v6 and PerMonitorV2 DPI awareness,
    // which is the fix for defect D2.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let script = write_resource_script();
        embed_resource::compile(script, embed_resource::NONE).manifest_required().unwrap();
    }
}

/// Exposes the git commit and build date as `env!` variables, replacing the
/// `<SourceRevisionId>20260123.2224</SourceRevisionId>` that was maintained by
/// hand in the csproj. Both degrade to `unknown` rather than failing the build,
/// because a release tarball has no git directory.
fn emit_build_provenance() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=LOCKSCREEN_COMMIT={commit}");

    // SOURCE_DATE_EPOCH if the build environment sets it, so a reproducible
    // build stays reproducible; otherwise the resource just says `unknown`
    // rather than baking in a timestamp that breaks bit-for-bit rebuilds.
    let date = std::env::var("SOURCE_DATE_EPOCH").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=LOCKSCREEN_BUILD_DATE={date}");
}

/// Writes the resource script the build compiles: `assets/app.rc` plus a
/// `VERSIONINFO` block, so Explorer's *Details* tab, Task Manager and any
/// installer all report the version the app was built with.
fn write_resource_script() -> std::path::PathBuf {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let assets =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("assets");
    // The script lands in OUT_DIR, so the files it names need full paths.
    // Forward slashes: the resource compiler accepts them and they need no
    // escaping inside a string.
    let dir = assets.display().to_string().replace('\\', "/");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let description = std::env::var("CARGO_PKG_DESCRIPTION").unwrap_or_default();
    // Shown as "Company" in the file properties and as the publisher in Task
    // Manager. `authors` in Cargo.toml records who wrote it; executables have
    // no author field of their own.
    let company = "Procybro Solutions";
    let copyright = "(C) 2026 Procybro Solutions";
    // FILEVERSION wants four numbers; a package version is three.
    let mut parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let (major, minor, patch, build) = (parts[0], parts[1], parts[2], parts[3]);

    let script = format!(
        r##"#define RT_MANIFEST 24

1 ICON "{dir}/app.ico"
1 RT_MANIFEST "{dir}/app.manifest"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},{build}
PRODUCTVERSION {major},{minor},{patch},{build}
FILEOS 0x4L
FILETYPE 0x1L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "{company}"
            VALUE "FileDescription", "{description}"
            VALUE "FileVersion", "{major}.{minor}.{patch}.{build}"
            VALUE "InternalName", "LockScreen"
            VALUE "LegalCopyright", "{copyright}"
            VALUE "OriginalFilename", "LockScreen.exe"
            VALUE "ProductName", "LockScreen"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        /* US English, Unicode. */
        VALUE "Translation", 0x409, 1200
    END
END
"##
    );

    let path = out.join("app-with-version.rc");
    let mut file = std::fs::File::create(&path).expect("writing the resource script");
    file.write_all(script.as_bytes()).expect("writing the resource script");
    path
}
