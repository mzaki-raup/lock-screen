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
        Screen { device: format!("\\\\.\\DISPLAY{left}"), bounds, work: bounds, primary }
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
