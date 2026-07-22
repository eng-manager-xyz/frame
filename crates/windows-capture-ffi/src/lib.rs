//! Audited Win32 boundary for Windows Graphics Capture.
//!
//! This crate alone owns pointer-level window enumeration and message-loop
//! control. The safe capture adapter receives opaque numeric identities and
//! coarse geometry; it cannot read a window title or process name.

#![deny(unsafe_op_in_unsafe_fn)]

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsCaptureFfiError;

impl fmt::Display for WindowsCaptureFfiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the Windows capture FFI operation failed")
    }
}

impl std::error::Error for WindowsCaptureFfiError {}

#[cfg(target_os = "windows")]
mod windows {
    use std::{
        mem,
        os::windows::io::AsRawHandle,
        sync::atomic::{AtomicBool, Ordering},
        thread::JoinHandle,
    };

    use wgc::{new_item_from_hwnd, new_item_from_monitor};
    use windows::{
        Graphics::Capture::GraphicsCaptureItem,
        Win32::{
            Foundation::{HANDLE, HWND, LPARAM, RECT, WPARAM},
            Graphics::{
                Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
                Gdi::{
                    DEVMODEW, DMDO_90, DMDO_180, DMDO_270, ENUM_CURRENT_SETTINGS,
                    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR,
                    MONITORINFO, MONITORINFOEXW,
                },
            },
            System::Threading::{GetCurrentProcessId, GetThreadId},
            UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            UI::WindowsAndMessaging::{
                EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, GetWindowThreadProcessId,
                IsWindowVisible, PostThreadMessageW, WM_QUIT, WS_CHILD, WS_EX_TOOLWINDOW,
            },
        },
        core::BOOL,
    };

    use super::WindowsCaptureFfiError;

    const MAX_ENUMERATED_DISPLAYS: usize = 257;
    const MAX_ENUMERATED_WINDOWS: usize = 257;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct NativeDisplay {
        native_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale_numerator: u32,
        scale_denominator: u32,
        rotation_degrees: u16,
    }

    impl NativeDisplay {
        #[must_use]
        pub const fn native_id(self) -> u64 {
            self.native_id
        }

        #[must_use]
        pub const fn x(self) -> i32 {
            self.x
        }

        #[must_use]
        pub const fn y(self) -> i32 {
            self.y
        }

        #[must_use]
        pub const fn width(self) -> u32 {
            self.width
        }

        #[must_use]
        pub const fn height(self) -> u32 {
            self.height
        }

        #[must_use]
        pub const fn scale_numerator(self) -> u32 {
            self.scale_numerator
        }

        #[must_use]
        pub const fn scale_denominator(self) -> u32 {
            self.scale_denominator
        }

        #[must_use]
        pub const fn rotation_degrees(self) -> u16 {
            self.rotation_degrees
        }
    }

    impl std::fmt::Debug for NativeDisplay {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("NativeDisplay")
                .field("native_id", &"<redacted>")
                .field("geometry", &"<redacted>")
                .finish()
        }
    }

    pub fn enumerate_displays() -> Result<Vec<NativeDisplay>, WindowsCaptureFfiError> {
        let mut displays = Vec::new();
        // SAFETY: `displays` stays alive and exclusively borrowed for the full
        // synchronous call. The callback never retains its pointer.
        if !unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(enumerate_monitor),
                LPARAM(std::ptr::addr_of_mut!(displays) as isize),
            )
        }
        .as_bool()
        {
            return Err(WindowsCaptureFfiError);
        }
        if displays.len() > MAX_ENUMERATED_DISPLAYS {
            return Err(WindowsCaptureFfiError);
        }
        Ok(displays)
    }

    unsafe extern "system" fn enumerate_monitor(
        monitor: HMONITOR,
        _device_context: HDC,
        _bounds: *mut RECT,
        parameter: LPARAM,
    ) -> BOOL {
        // SAFETY: `parameter` is the live exclusive Vec pointer supplied by
        // `enumerate_displays` for this synchronous call.
        let displays = unsafe { &mut *(parameter.0 as *mut Vec<NativeDisplay>) };
        if displays.len() >= MAX_ENUMERATED_DISPLAYS {
            return BOOL(1);
        }
        if let Ok(display) = inspect_monitor(monitor) {
            displays.push(display);
        }
        BOOL(1)
    }

    fn inspect_monitor(monitor: HMONITOR) -> Result<NativeDisplay, ()> {
        let mut info = MONITORINFOEXW {
            monitorInfo: MONITORINFO {
                cbSize: u32::try_from(mem::size_of::<MONITORINFOEXW>()).map_err(|_| ())?,
                ..MONITORINFO::default()
            },
            szDevice: [0; 32],
        };
        // SAFETY: `info` advertises its exact initialized size and remains a
        // valid writable MONITORINFOEXW for the duration of this call.
        unsafe { GetMonitorInfoW(monitor, std::ptr::addr_of_mut!(info).cast::<MONITORINFO>()) }
            .ok()
            .map_err(|_| ())?;
        let rect = info.monitorInfo.rcMonitor;
        let width = u32::try_from(rect.right.checked_sub(rect.left).ok_or(())?).map_err(|_| ())?;
        let height = u32::try_from(rect.bottom.checked_sub(rect.top).ok_or(())?).map_err(|_| ())?;
        if width == 0 || height == 0 {
            return Err(());
        }
        let mut dpi_x = 0_u32;
        let mut dpi_y = 0_u32;
        // SAFETY: both DPI outputs are valid writable u32 values.
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
            .map_err(|_| ())?;
        if dpi_x == 0 || dpi_y == 0 || dpi_x != dpi_y {
            return Err(());
        }
        let mut mode = DEVMODEW {
            dmSize: u16::try_from(mem::size_of::<DEVMODEW>()).map_err(|_| ())?,
            ..DEVMODEW::default()
        };
        // SAFETY: the NUL-terminated device array belongs to `info`, and
        // `mode` advertises its exact initialized size.
        if !unsafe {
            EnumDisplaySettingsW(
                windows::core::PCWSTR(info.szDevice.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut mode,
            )
        }
        .as_bool()
        {
            return Err(());
        }
        // SAFETY: EnumDisplaySettingsW initialized the display-mode union.
        let orientation = unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation };
        let rotation_degrees = match orientation {
            DMDO_90 => 90,
            DMDO_180 => 180,
            DMDO_270 => 270,
            _ => 0,
        };
        let native_id = u64::try_from(monitor.0.addr()).map_err(|_| ())?;
        Ok(NativeDisplay {
            native_id,
            x: rect.left,
            y: rect.top,
            width,
            height,
            scale_numerator: dpi_x,
            scale_denominator: 96,
            rotation_degrees,
        })
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct NativeWindow {
        native_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    }

    impl NativeWindow {
        #[must_use]
        pub const fn native_id(self) -> u64 {
            self.native_id
        }

        #[must_use]
        pub const fn x(self) -> i32 {
            self.x
        }

        #[must_use]
        pub const fn y(self) -> i32 {
            self.y
        }

        #[must_use]
        pub const fn width(self) -> u32 {
            self.width
        }

        #[must_use]
        pub const fn height(self) -> u32 {
            self.height
        }
    }

    impl std::fmt::Debug for NativeWindow {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("NativeWindow")
                .field("native_id", &"<redacted>")
                .field("geometry", &"<redacted>")
                .finish()
        }
    }

    struct EnumerationContext {
        current_pid: u32,
        failed: AtomicBool,
        windows: Vec<NativeWindow>,
    }

    pub fn enumerate_non_frame_windows() -> Result<Vec<NativeWindow>, WindowsCaptureFfiError> {
        let mut context = EnumerationContext {
            // SAFETY: this process-local query has no pointer preconditions.
            current_pid: unsafe { GetCurrentProcessId() },
            failed: AtomicBool::new(false),
            windows: Vec::new(),
        };
        // SAFETY: `context` stays alive and exclusively borrowed for the full
        // synchronous EnumWindows call. The callback reconstructs only this
        // exact pointer and never retains it.
        unsafe {
            EnumWindows(
                Some(enumerate_window),
                LPARAM(std::ptr::addr_of_mut!(context) as isize),
            )
        }
        .map_err(|_| WindowsCaptureFfiError)?;
        if context.failed.load(Ordering::Acquire) {
            return Err(WindowsCaptureFfiError);
        }
        Ok(context.windows)
    }

    unsafe extern "system" fn enumerate_window(hwnd: HWND, parameter: LPARAM) -> BOOL {
        // SAFETY: `parameter` is the live exclusive EnumerationContext pointer
        // supplied by `enumerate_non_frame_windows` for this synchronous call.
        let context = unsafe { &mut *(parameter.0 as *mut EnumerationContext) };
        if context.windows.len() >= MAX_ENUMERATED_WINDOWS {
            return BOOL(1);
        }
        match inspect_window(hwnd, context.current_pid) {
            Ok(Some(window)) => context.windows.push(window),
            Ok(None) => {}
            Err(()) => context.failed.store(true, Ordering::Release),
        }
        BOOL(1)
    }

    fn inspect_window(hwnd: HWND, current_pid: u32) -> Result<Option<NativeWindow>, ()> {
        // SAFETY: HWND is supplied by EnumWindows and valid for the duration of
        // this callback. These queries do not retain output pointers.
        if !unsafe { IsWindowVisible(hwnd).as_bool() } {
            return Ok(None);
        }
        let mut pid = 0_u32;
        // SAFETY: `pid` is a valid writable u32 for this call.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 || pid == current_pid {
            return Ok(None);
        }
        // SAFETY: style reads are side-effect free for the enumerated HWND.
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
        // SAFETY: style reads are side-effect free for the enumerated HWND.
        let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        if style & isize::try_from(WS_CHILD.0).map_err(|_| ())? != 0
            || extended_style & isize::try_from(WS_EX_TOOLWINDOW.0).map_err(|_| ())? != 0
        {
            return Ok(None);
        }
        let mut cloaked = 0_u32;
        // SAFETY: `cloaked` and its exact byte length are valid for the DWM query.
        if unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                std::ptr::addr_of_mut!(cloaked).cast(),
                u32::try_from(mem::size_of::<u32>()).map_err(|_| ())?,
            )
        }
        .is_ok()
            && cloaked != 0
        {
            return Ok(None);
        }
        let mut rect = RECT::default();
        // SAFETY: `rect` and its exact byte length are valid for the DWM query.
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                std::ptr::addr_of_mut!(rect).cast(),
                u32::try_from(mem::size_of::<RECT>()).map_err(|_| ())?,
            )
        }
        .map_err(|_| ())?;
        let width = u32::try_from(rect.right.checked_sub(rect.left).ok_or(())?).map_err(|_| ())?;
        let height = u32::try_from(rect.bottom.checked_sub(rect.top).ok_or(())?).map_err(|_| ())?;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let native_id = u64::try_from(hwnd.0.addr()).map_err(|_| ())?;
        Ok(Some(NativeWindow {
            native_id,
            x: rect.left,
            y: rect.top,
            width,
            height,
        }))
    }

    pub fn capture_item_for_monitor(
        native_id: u64,
    ) -> Result<GraphicsCaptureItem, WindowsCaptureFfiError> {
        let address = usize::try_from(native_id).map_err(|_| WindowsCaptureFfiError)?;
        new_item_from_monitor(HMONITOR(address as *mut std::ffi::c_void))
            .map_err(|_| WindowsCaptureFfiError)
    }

    pub fn capture_item_for_window(
        native_id: u64,
    ) -> Result<GraphicsCaptureItem, WindowsCaptureFfiError> {
        let address = usize::try_from(native_id).map_err(|_| WindowsCaptureFfiError)?;
        new_item_from_hwnd(HWND(address as *mut std::ffi::c_void))
            .map_err(|_| WindowsCaptureFfiError)
    }

    pub fn request_worker_stop<T>(worker: &JoinHandle<T>) -> Result<(), WindowsCaptureFfiError> {
        let raw = worker.as_raw_handle();
        // SAFETY: the borrowed JoinHandle keeps its OS thread handle alive.
        let thread_id = unsafe { GetThreadId(HANDLE(raw)) };
        if thread_id == 0 {
            return Err(WindowsCaptureFfiError);
        }
        // SAFETY: the WGC worker creates a message queue before publishing its
        // startup result. WM_QUIT carries no borrowed pointer payload.
        unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM::default(), LPARAM::default()) }
            .map_err(|_| WindowsCaptureFfiError)
    }
}

#[cfg(target_os = "windows")]
pub use windows::{
    NativeDisplay, NativeWindow, capture_item_for_monitor, capture_item_for_window,
    enumerate_displays, enumerate_non_frame_windows, request_worker_stop,
};
