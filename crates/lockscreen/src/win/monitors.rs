//! Asking Windows which monitors are attached, and where each one is.
//!
//! Ported from `flex-taskbar/src/win/monitors.rs`. Every coordinate here is a
//! **physical pixel**, which is the whole of defect D2: the .NET version takes
//! `Screen.Bounds` (physical) and assigns it to WPF's `Left`/`Top`/`Width`/
//! `Height` (device-independent), so on a 150%-scaled monitor the overlay comes
//! out two thirds of the size it should be and leaves the desktop showing. The
//! manifest declares `PerMonitorV2`, so these numbers are real pixels and there
//! is no second unit anywhere to confuse them with.

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::BOOL;

/// One attached monitor.
///
/// Only `PartialEq`, not `Eq`: `RECT` is a plain Win32 struct and does not
/// implement `Eq`, and deriving it by hand would buy nothing here.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// `\\.\DISPLAY1` and friends. Only used for diagnostics, but a log line
    /// that names the screen is worth a great deal when a report says "the
    /// overlay missed one".
    pub device: String,
    /// The whole monitor, in physical pixels. The overlay covers this, not
    /// `work`: a lock screen that leaves the taskbar uncovered is not a lock
    /// screen.
    pub bounds: RECT,
    /// The part not covered by the taskbar and other appbars.
    pub work: RECT,
    pub primary: bool,
    /// Kept so [`Screen::dpi`] can ask about this monitor specifically rather
    /// than re-deriving a handle from a coordinate.
    pub handle: HMONITOR,
}

impl Screen {
    pub fn width(&self) -> i32 {
        self.bounds.right - self.bounds.left
    }

    pub fn height(&self) -> i32 {
        self.bounds.bottom - self.bounds.top
    }

    /// Whether a point in virtual-desktop coordinates falls on this monitor.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.bounds.left && x < self.bounds.right && y >= self.bounds.top && y < self.bounds.bottom
    }

    /// This monitor's effective DPI, or 96 if Windows will not say.
    ///
    /// Per-monitor, not per-process: on a mixed-DPI desktop these differ, and
    /// sizing a window with the wrong one is defect D2. The overlay itself needs
    /// no scaling -- it is a fill measured in physical pixels -- but the prompt
    /// drawn over it does, and that takes its DPI from the window it is parented
    /// to rather than from here.
    pub fn dpi(&self) -> u32 {
        use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        let mut x = 0;
        let mut y = 0;
        // SAFETY: `self.handle` came from EnumDisplayMonitors, and both
        // out-parameters are live for the call.
        let ok = unsafe { GetDpiForMonitor(self.handle, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
        if ok.is_ok() && x > 0 { x } else { 96 }
    }
}

fn describe(handle: HMONITOR) -> Option<Screen> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO { cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32, ..Default::default() },
        ..Default::default()
    };
    // GetMonitorInfoW reads cbSize to tell MONITORINFO and MONITORINFOEXW apart,
    // which is why it is set to the larger size above.
    //
    // SAFETY: `info` is a correctly sized, initialised structure and the handle
    // comes from the enumeration callback.
    let ok = unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo as *mut MONITORINFO) };
    if !ok.as_bool() {
        return None;
    }
    let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
    Some(Screen {
        device: String::from_utf16_lossy(&info.szDevice[..len]),
        bounds: info.monitorInfo.rcMonitor,
        work: info.monitorInfo.rcWork,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        handle,
    })
}

unsafe extern "system" fn collect(handle: HMONITOR, _hdc: HDC, _clip: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<Screen>` handed to EnumDisplayMonitors
    // below, which outlives the enumeration.
    let out = unsafe { &mut *(data.0 as *mut Vec<Screen>) };
    if let Some(screen) = describe(handle) {
        out.push(screen);
    }
    // Keep going: a lock screen needs every monitor, not the first one.
    BOOL(1)
}

/// Every attached monitor, ordered left to right and then top to bottom.
pub fn all() -> Vec<Screen> {
    let mut out: Vec<Screen> = Vec::new();
    // SAFETY: the callback only writes through `data`, which points at `out`
    // for the duration of the call.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut out as *mut _ as isize));
    }
    out.sort_by_key(|s| (s.bounds.left, s.bounds.top));
    out
}

/// Index of the monitor a point falls on, or of the primary one if it falls on
/// none -- which happens when a monitor is unplugged while the pointer is still
/// reported over where it used to be.
///
/// An index rather than a reference because the caller pairs it with a parallel
/// list of overlay windows.
pub fn index_at(screens: &[Screen], x: i32, y: i32) -> Option<usize> {
    screens
        .iter()
        .position(|s| s.contains(x, y))
        .or_else(|| screens.iter().position(|s| s.primary))
        .or_else(|| (!screens.is_empty()).then_some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(left: i32, top: i32, w: i32, h: i32, primary: bool) -> Screen {
        let bounds = RECT { left, top, right: left + w, bottom: top + h };
        Screen { device: format!("\\\\.\\DISPLAY{left}"), bounds, work: bounds, primary, handle: HMONITOR::default() }
    }

    #[test]
    fn this_machine_reports_at_least_one_monitor_with_a_sane_size() {
        let screens = all();
        assert!(!screens.is_empty(), "a running desktop session has at least one monitor");
        assert_eq!(screens.iter().filter(|s| s.primary).count(), 1, "exactly one monitor is primary");
        for s in &screens {
            assert!(s.width() > 0 && s.height() > 0, "{s:?}");
            // Physical pixels. A monitor narrower than 640 would mean the DPI
            // units got mixed up somewhere, which is exactly defect D2.
            assert!(s.width() >= 640, "suspiciously narrow, check DPI awareness: {s:?}");
        }
    }

    /// Prints the real layout, including per-monitor DPI. Ignored because it
    /// asserts nothing; run it deliberately when a report mentions a monitor
    /// being missed or mis-sized:
    ///
    /// ```text
    /// cargo test -p lockscreen --bin LockScreen layout -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "diagnostic; prints the monitor layout"]
    fn layout() {
        for s in all() {
            println!(
                "{:<14} {:>5}x{:<5} at ({:>6},{:>6})  dpi {:>3} ({}%){}",
                s.device,
                s.width(),
                s.height(),
                s.bounds.left,
                s.bounds.top,
                s.dpi(),
                s.dpi() * 100 / 96,
                if s.primary { "  primary" } else { "" }
            );
        }
    }

    /// What the same enumeration looks like to a **DPI-unaware** thread, which
    /// is what the .NET app is.
    ///
    /// `LockScreenApp.exe` has no DPI declaration at all -- the manifest the
    /// .NET SDK embeds in it carries only a `requestedExecutionLevel` -- so
    /// Windows virtualises coordinates for it. This runs `all()` twice, once
    /// under each awareness, and returns the pair. It is the measurement behind
    /// defect D2.
    fn both_awarenesses() -> (Vec<Screen>, Vec<Screen>) {
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_UNAWARE, SetThreadDpiAwarenessContext,
        };
        // SAFETY: both contexts are documented constants, and the thread's
        // original context is put back before returning.
        unsafe {
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE);
            let unaware = all();
            SetThreadDpiAwarenessContext(if previous.is_invalid() {
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
            } else {
                previous
            });
            (unaware, all())
        }
    }

    /// Prints what each awareness sees. Ignored because the interesting output
    /// depends on the hardware: on a single-DPI desktop the two agree, which is
    /// precisely why D2 went unnoticed for so long.
    ///
    /// ```text
    /// cargo test -p lockscreen --bin LockScreen dpi_virtualisation -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "diagnostic; prints what each DPI awareness sees"]
    fn dpi_virtualisation() {
        let (unaware, aware) = both_awarenesses();
        println!("{:<14} {:>12} {:>12} {:>6}", "device", "unaware", "per-monitor", "dpi");
        for screen in &aware {
            let same = unaware.iter().find(|u| u.device == screen.device);
            println!(
                "{:<14} {:>12} {:>12} {:>6}",
                screen.device,
                same.map(|u| format!("{}x{}", u.width(), u.height())).unwrap_or_else(|| "-".to_owned()),
                format!("{}x{}", screen.width(), screen.height()),
                screen.dpi()
            );
        }
    }

    /// The manifest regression test, and the one that would have caught D2.
    ///
    /// Two assertions, both hardware-independent in form:
    ///
    /// - a DPI-unaware thread never sees a monitor as *larger* than a
    ///   per-monitor-aware one does, because virtualisation only ever scales
    ///   down -- `physical * 96 / dpi`;
    /// - on a monitor scaled above 100% the two views must **differ**. If they
    ///   agree, this process is not per-monitor aware after all, which means
    ///   `app.manifest` stopped being embedded -- a silent failure that turns
    ///   every coordinate in the overlay into a virtualised one.
    ///
    /// The second assertion is vacuous on a desktop where every monitor runs at
    /// 100%, and live on one that does not. That is the whole story of D2: the
    /// bug is invisible on the hardware most people develop on.
    #[test]
    fn this_process_really_is_per_monitor_aware() {
        let (unaware, aware) = both_awarenesses();
        assert_eq!(unaware.len(), aware.len(), "the same monitors should be enumerated either way");

        let mut scaled = 0;
        for screen in &aware {
            let Some(same) = unaware.iter().find(|u| u.device == screen.device) else {
                panic!("{} was missing from the unaware enumeration", screen.device);
            };
            assert!(
                same.width() <= screen.width() && same.height() <= screen.height(),
                "virtualisation should only ever scale down: {} unaware {}x{} vs aware {}x{}",
                screen.device,
                same.width(),
                same.height(),
                screen.width(),
                screen.height()
            );

            if screen.dpi() > 96 {
                scaled += 1;
                assert_ne!(
                    (same.width(), same.height()),
                    (screen.width(), screen.height()),
                    "{} runs at {}% but both awarenesses agree, so this process is not per-monitor aware -- \
                     check that app.manifest is still being embedded",
                    screen.device,
                    screen.dpi() * 100 / 96
                );
                // What the .NET app hands WPF for this monitor, measured rather
                // than argued: the unaware view, which is smaller.
                assert_eq!(
                    same.width(),
                    screen.width() * 96 / screen.dpi() as i32,
                    "the unaware width should be the physical one scaled by 96/dpi"
                );
            }
        }

        if scaled == 0 {
            // Not a failure: it only means this machine cannot demonstrate D2.
            // Said out loud so nobody reads a pass here as proof of more than
            // it is.
            println!("every monitor runs at 100%, so the per-monitor assertions above were vacuous");
        }
    }

    #[test]
    fn every_monitor_reports_a_plausible_dpi() {
        for s in all() {
            let dpi = s.dpi();
            assert!((96..=960).contains(&dpi), "{}: implausible dpi {dpi}", s.device);
        }
    }

    #[test]
    fn bounds_cover_at_least_as_much_as_the_work_area() {
        for s in all() {
            assert!(s.bounds.left <= s.work.left && s.bounds.top <= s.work.top, "{s:?}");
            assert!(s.bounds.right >= s.work.right && s.bounds.bottom >= s.work.bottom, "{s:?}");
        }
    }

    #[test]
    fn contains_is_half_open_so_adjacent_monitors_do_not_overlap() {
        let left = screen(0, 0, 1920, 1080, true);
        let right = screen(1920, 0, 1920, 1080, false);
        assert!(left.contains(0, 0));
        assert!(left.contains(1919, 1079));
        assert!(!left.contains(1920, 0), "the right edge belongs to the next monitor");
        assert!(right.contains(1920, 0));
        assert!(!right.contains(3840, 0));
    }

    #[test]
    fn a_point_off_every_monitor_falls_back_to_the_primary() {
        let screens = vec![screen(0, 0, 1920, 1080, false), screen(1920, 0, 1920, 1080, true)];
        assert_eq!(index_at(&screens, 2000, 500), Some(1), "a point on the second monitor");
        assert_eq!(index_at(&screens, 100, 100), Some(0), "a point on the first");
        // Off the desktop entirely: the primary, not None, so the prompt always
        // has somewhere to go.
        assert_eq!(index_at(&screens, -5000, -5000), Some(1), "the primary is index 1 here");
        assert_eq!(index_at(&[], 0, 0), None, "but no monitors means no answer");
    }

    #[test]
    fn with_no_primary_flagged_the_first_monitor_is_used() {
        let screens = vec![screen(0, 0, 1920, 1080, false), screen(1920, 0, 1920, 1080, false)];
        assert_eq!(index_at(&screens, -5000, -5000), Some(0));
    }
}
