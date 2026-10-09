//! The lock overlay. Phase 01.
//!
//! Will create one layered window per monitor
//! (`WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE`), size them from
//! `EnumDisplayMonitors` in physical pixels, blur the desktop behind via
//! `DWMWA_SYSTEMBACKDROP_TYPE` on Windows 11 or
//! `ACCENT_ENABLE_ACRYLICBLURBEHIND` on Windows 10 1809+, install the
//! `WH_KEYBOARD_LL` / `WH_MOUSE_LL` hooks (phase 02) and hold the Task Manager
//! policy guard for as long as it is up.

use std::path::Path;

use lockscreen_core::config::Config;

pub fn run(_config_path: &Path, config: Config) -> Result<(), String> {
    Err(format!(
        "The lock overlay is not implemented yet (phase 01).\n\n\
         Configuration loaded successfully:\n\
         \u{2022} title: {}\n\
         \u{2022} transparency: {:.2}\n\
         \u{2022} blur: {}\n\
         \u{2022} voice unlock: {}",
        config.title,
        config.transparency,
        if config.blur_background { "on" } else { "off" },
        if config.voice.enabled { "on" } else { "off" },
    ))
}
