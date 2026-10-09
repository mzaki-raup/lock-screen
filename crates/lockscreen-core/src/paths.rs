//! Where the app's files live: beside the executable, always.
//!
//! This module exists because "beside the executable" and "the working
//! directory" are different directories, and the .NET version conflates them.
//! `IniHelper` uses the bare relative path `"config.ini"`, which resolves
//! against the process's current working directory -- so a shortcut with a
//! different *Start in* field, a scheduled task, or a launch from a terminal
//! elsewhere each read a different file. Finding none, it writes a fresh one
//! carrying the default password into whatever directory that happened to be.
//!
//! So: resolve once from [`GetModuleFileNameW`], take the parent, and never
//! call `std::env::current_dir()` anywhere in this program.
//!
//! [`GetModuleFileNameW`]: https://learn.microsoft.com/windows/win32/api/libloaderapi/nf-libloaderapi-getmodulefilenamew

use std::io;
use std::path::{Path, PathBuf};

/// The settings file, beside the executable. The only file holding user intent.
pub const CONFIG_FILE: &str = "config.ini";

/// Derived state -- update downloads, logs. Safe to delete at any time.
pub const DATA_DIR: &str = "data";

/// The directory the running executable sits in.
///
/// # Errors
///
/// Fails only if the OS cannot report the module path, which in practice means
/// something is very wrong with the process.
#[cfg(windows)]
pub fn exe_dir() -> io::Result<PathBuf> {
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;

    // MAX_PATH is not the limit on a long-path-enabled system, so grow until the
    // name fits rather than assuming 260. GetModuleFileNameW signals truncation
    // by returning exactly the buffer length (and setting ERROR_INSUFFICIENT_BUFFER).
    let mut buf = vec![0u16; 512];
    loop {
        let len = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len < buf.len() {
            let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
            return path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| io::Error::other("the executable path has no parent directory"));
        }
        if buf.len() >= 64 * 1024 {
            return Err(io::Error::other("the executable path is implausibly long"));
        }
        buf.resize(buf.len() * 2, 0);
    }
}

/// Off Windows there is no module handle; the current directory is the best
/// available answer and is only ever used by tests.
#[cfg(not(windows))]
pub fn exe_dir() -> io::Result<PathBuf> {
    std::env::current_dir()
}

/// Full path to `config.ini`, beside the executable.
pub fn config_path() -> io::Result<PathBuf> {
    Ok(exe_dir()?.join(CONFIG_FILE))
}

/// Full path to the derived-state directory, beside the executable. Not created.
pub fn data_dir() -> io::Result<PathBuf> {
    Ok(exe_dir()?.join(DATA_DIR))
}

/// Whether the app's own directory can be written to.
///
/// A portable app placed in `%ProgramFiles%` cannot save its settings, and the
/// one outcome to avoid is discovering that at save time. Call this at startup
/// and say so plainly; the updater should also check it before downloading
/// anything.
///
/// Probes by creating and removing a uniquely named file, because directory
/// permissions on Windows are not reliably inferable from metadata.
pub fn is_dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".lockscreen-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_sits_beside_the_executable() {
        let dir = exe_dir().expect("an executable directory");
        let config = config_path().expect("a config path");
        assert_eq!(config.parent(), Some(dir.as_path()));
        assert_eq!(config.file_name().and_then(|n| n.to_str()), Some(CONFIG_FILE));
    }

    #[test]
    fn data_dir_sits_beside_the_executable() {
        let dir = exe_dir().expect("an executable directory");
        assert_eq!(data_dir().expect("a data path").parent(), Some(dir.as_path()));
    }

    #[test]
    fn a_temp_dir_is_writable_and_a_missing_one_is_not() {
        assert!(is_dir_writable(&std::env::temp_dir()));
        assert!(!is_dir_writable(Path::new("Z:/no/such/directory/anywhere")));
    }
}
