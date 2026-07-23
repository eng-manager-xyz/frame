//! Safe, bounded cursor metadata over the Win32 pointer boundary.

use std::{ffi::c_void, mem, ptr, slice};

use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetObjectW, HBITMAP, HDC, HGDIOBJ,
        ReleaseDC, SelectObject,
    },
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
        WindowsAndMessaging::{
            CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HCURSOR,
            HICON, ICONINFO,
        },
    },
};

use crate::WindowsCaptureFfiError;

const MAX_CURSOR_DIMENSION: u16 = 256;
const BYTES_PER_PIXEL: usize = 4;

pub struct WindowsCursorSampler {
    last_cursor: Option<usize>,
    revision: u64,
}

pub struct WindowsCursorSample {
    visible: bool,
    desktop_x: i32,
    desktop_y: i32,
    primary_click: bool,
    secondary_click: bool,
    image_revision: Option<u64>,
    changed_image: Option<WindowsCursorImage>,
}

pub struct WindowsCursorImage {
    revision: u64,
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    bgra: Box<[u8]>,
}

impl WindowsCursorSampler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_cursor: None,
            revision: 0,
        }
    }

    pub fn sample(
        &mut self,
        include_image: bool,
    ) -> Result<WindowsCursorSample, WindowsCaptureFfiError> {
        let mut info = CURSORINFO {
            cbSize: u32::try_from(mem::size_of::<CURSORINFO>())
                .map_err(|_| WindowsCaptureFfiError)?,
            ..CURSORINFO::default()
        };
        // SAFETY: `info` is an initialized, exactly sized writable CURSORINFO.
        unsafe { GetCursorInfo(ptr::addr_of_mut!(info)) }.map_err(|_| WindowsCaptureFfiError)?;
        let visible = info.flags == CURSOR_SHOWING && !info.hCursor.is_invalid();
        let primary_click = visible && button_pressed(i32::from(VK_LBUTTON.0));
        let secondary_click = visible && button_pressed(i32::from(VK_RBUTTON.0));

        if !visible || !include_image {
            return Ok(WindowsCursorSample {
                visible,
                desktop_x: info.ptScreenPos.x,
                desktop_y: info.ptScreenPos.y,
                primary_click,
                secondary_click,
                image_revision: include_image
                    .then_some(self.revision)
                    .filter(|revision| *revision > 0),
                changed_image: None,
            });
        }

        let cursor_identity = info.hCursor.0.addr();
        let changed_image = if self.last_cursor == Some(cursor_identity) {
            None
        } else {
            let revision = self.revision.checked_add(1).ok_or(WindowsCaptureFfiError)?;
            let image = capture_cursor_image(info.hCursor, revision)?;
            self.last_cursor = Some(cursor_identity);
            self.revision = revision;
            Some(image)
        };
        Ok(WindowsCursorSample {
            visible,
            desktop_x: info.ptScreenPos.x,
            desktop_y: info.ptScreenPos.y,
            primary_click,
            secondary_click,
            image_revision: Some(self.revision),
            changed_image,
        })
    }
}

impl Default for WindowsCursorSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsCursorSample {
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub const fn desktop_position(&self) -> (i32, i32) {
        (self.desktop_x, self.desktop_y)
    }

    #[must_use]
    pub const fn primary_click(&self) -> bool {
        self.primary_click
    }

    #[must_use]
    pub const fn secondary_click(&self) -> bool {
        self.secondary_click
    }

    #[must_use]
    pub const fn image_revision(&self) -> Option<u64> {
        self.image_revision
    }

    #[must_use]
    pub fn take_changed_image(&mut self) -> Option<WindowsCursorImage> {
        self.changed_image.take()
    }
}

impl WindowsCursorImage {
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn hotspot(&self) -> (u16, u16) {
        (self.hotspot_x, self.hotspot_y)
    }

    #[must_use]
    pub fn into_bgra(self) -> Box<[u8]> {
        self.bgra
    }
}

fn button_pressed(key: i32) -> bool {
    // SAFETY: GetAsyncKeyState accepts every representable virtual-key value;
    // the constants supplied here are valid mouse-button keys.
    (unsafe { GetAsyncKeyState(key) } as u16) & 0x8000 != 0
}

fn capture_cursor_image(
    cursor: HCURSOR,
    revision: u64,
) -> Result<WindowsCursorImage, WindowsCaptureFfiError> {
    let mut icon = ICONINFO::default();
    // SAFETY: `icon` is a valid writable ICONINFO. Successful GetIconInfo
    // transfers two bitmap handles which are released below on every path.
    unsafe { GetIconInfo(HICON(cursor.0), ptr::addr_of_mut!(icon)) }
        .map_err(|_| WindowsCaptureFfiError)?;
    let result = capture_icon_pixels(&icon, cursor, revision);
    let color_deleted = delete_bitmap(icon.hbmColor);
    let mask_deleted = delete_bitmap(icon.hbmMask);
    if !color_deleted || !mask_deleted {
        return Err(WindowsCaptureFfiError);
    }
    result
}

fn capture_icon_pixels(
    icon: &ICONINFO,
    cursor: HCURSOR,
    revision: u64,
) -> Result<WindowsCursorImage, WindowsCaptureFfiError> {
    let bitmap_handle = if icon.hbmColor.is_invalid() {
        icon.hbmMask
    } else {
        icon.hbmColor
    };
    if bitmap_handle.is_invalid() {
        return Err(WindowsCaptureFfiError);
    }
    let mut bitmap = BITMAP::default();
    // SAFETY: `bitmap` is an exactly sized writable BITMAP and the icon-owned
    // handle stays live until this function returns.
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap_handle.0),
            i32::try_from(mem::size_of::<BITMAP>()).map_err(|_| WindowsCaptureFfiError)?,
            Some(ptr::addr_of_mut!(bitmap).cast::<c_void>()),
        )
    };
    if copied == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
        return Err(WindowsCaptureFfiError);
    }
    let width = u16::try_from(bitmap.bmWidth).map_err(|_| WindowsCaptureFfiError)?;
    let native_height = if icon.hbmColor.is_invalid() {
        bitmap
            .bmHeight
            .checked_div(2)
            .ok_or(WindowsCaptureFfiError)?
    } else {
        bitmap.bmHeight
    };
    let height = u16::try_from(native_height).map_err(|_| WindowsCaptureFfiError)?;
    if width == 0 || height == 0 || width > MAX_CURSOR_DIMENSION || height > MAX_CURSOR_DIMENSION {
        return Err(WindowsCaptureFfiError);
    }
    let hotspot_x = u16::try_from(icon.xHotspot).map_err(|_| WindowsCaptureFfiError)?;
    let hotspot_y = u16::try_from(icon.yHotspot).map_err(|_| WindowsCaptureFfiError)?;
    if hotspot_x >= width || hotspot_y >= height {
        return Err(WindowsCaptureFfiError);
    }
    let byte_count = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
        .ok_or(WindowsCaptureFfiError)?;

    // SAFETY: a null HWND requests the desktop DC and retains no Rust pointer.
    let screen_dc = unsafe { GetDC(None) };
    if screen_dc.is_invalid() {
        return Err(WindowsCaptureFfiError);
    }
    let result = capture_with_screen_dc(
        screen_dc, cursor, revision, width, height, hotspot_x, hotspot_y, byte_count,
    );
    // SAFETY: `screen_dc` came from GetDC(None) in this function.
    let released = unsafe { ReleaseDC(None, screen_dc) } == 1;
    if !released {
        return Err(WindowsCaptureFfiError);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn capture_with_screen_dc(
    screen_dc: HDC,
    cursor: HCURSOR,
    revision: u64,
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    byte_count: usize,
) -> Result<WindowsCursorImage, WindowsCaptureFfiError> {
    // SAFETY: `screen_dc` is live for this call and no pointer is retained.
    let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
    if memory_dc.is_invalid() {
        return Err(WindowsCaptureFfiError);
    }
    let result = capture_with_memory_dc(
        memory_dc, cursor, revision, width, height, hotspot_x, hotspot_y, byte_count,
    );
    // SAFETY: `memory_dc` came from CreateCompatibleDC and has no selected
    // owned bitmap after capture_with_memory_dc returns.
    let deleted = unsafe { DeleteDC(memory_dc) }.as_bool();
    if !deleted {
        return Err(WindowsCaptureFfiError);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn capture_with_memory_dc(
    memory_dc: HDC,
    cursor: HCURSOR,
    revision: u64,
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    byte_count: usize,
) -> Result<WindowsCursorImage, WindowsCaptureFfiError> {
    let header = BITMAPINFOHEADER {
        biSize: u32::try_from(mem::size_of::<BITMAPINFOHEADER>())
            .map_err(|_| WindowsCaptureFfiError)?,
        biWidth: i32::from(width),
        biHeight: -i32::from(height),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..BITMAPINFOHEADER::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        bmiColors: [Default::default()],
    };
    let mut bits = ptr::null_mut::<c_void>();
    // SAFETY: `info` and `bits` are valid for the synchronous call. The
    // returned bitmap owns the storage until DeleteObject below.
    let dib = unsafe {
        CreateDIBSection(
            Some(memory_dc),
            ptr::addr_of!(info),
            DIB_RGB_COLORS,
            ptr::addr_of_mut!(bits),
            None,
            0,
        )
    }
    .map_err(|_| WindowsCaptureFfiError)?;
    if bits.is_null() {
        let _ = delete_bitmap(dib);
        return Err(WindowsCaptureFfiError);
    }
    // SAFETY: `bits` points to the DIB's exact width*height*4 allocation.
    unsafe { ptr::write_bytes(bits, 0, byte_count) };
    // SAFETY: both handles are live; SelectObject returns the prior borrowed
    // object which must be restored before deleting `dib`.
    let previous = unsafe { SelectObject(memory_dc, HGDIOBJ(dib.0)) };
    if previous.is_invalid() {
        let _ = delete_bitmap(dib);
        return Err(WindowsCaptureFfiError);
    }
    // SAFETY: the DIB is selected in `memory_dc`, dimensions are bounded, and
    // the cursor handle remains live for this synchronous draw.
    let draw = unsafe {
        DrawIconEx(
            memory_dc,
            0,
            0,
            HICON(cursor.0),
            i32::from(width),
            i32::from(height),
            0,
            None,
            DI_NORMAL,
        )
    };
    let pixels = if draw.is_ok() {
        // SAFETY: `bits` remains live and initialized through the selected DIB.
        Some(unsafe { slice::from_raw_parts(bits.cast::<u8>(), byte_count) }.to_vec())
    } else {
        None
    };
    // SAFETY: `previous` came from SelectObject on this exact DC.
    let restored = unsafe { SelectObject(memory_dc, previous) };
    let deleted = delete_bitmap(dib);
    if restored.is_invalid() || !deleted {
        return Err(WindowsCaptureFfiError);
    }
    let bgra = pixels.ok_or(WindowsCaptureFfiError)?.into_boxed_slice();
    Ok(WindowsCursorImage {
        revision,
        width,
        height,
        hotspot_x,
        hotspot_y,
        bgra,
    })
}

fn delete_bitmap(bitmap: HBITMAP) -> bool {
    if bitmap.is_invalid() {
        return true;
    }
    // SAFETY: each bitmap passed here is owned by this module and deleted once.
    unsafe { DeleteObject(HGDIOBJ(bitmap.0)) }.as_bool()
}
