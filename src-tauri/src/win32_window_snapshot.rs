#![cfg(windows)]

use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
            GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
            SRCCOPY,
        },
    },
    Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS},
    UI::WindowsAndMessaging::{GetWindowRect, IsIconic},
};

fn rect_dimensions(rect: &RECT) -> Option<(u32, u32)> {
    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    if width <= 0 || height <= 0 {
        return None;
    }
    Some((width as u32, height as u32))
}

pub fn get_window_physical_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    let rect_size = std::mem::size_of::<RECT>() as u32;
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            rect_size,
        )
    }
    .is_ok()
        && rect_dimensions(&rect).is_some()
    {
        return Some(rect);
    }

    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() && rect_dimensions(&rect).is_some() {
        return Some(rect);
    }

    None
}

pub fn get_window_physical_dimensions(hwnd: HWND) -> (u64, u64) {
    let Some(rect) = get_window_physical_rect(hwnd) else {
        return (0, 0);
    };
    rect_dimensions(&rect)
        .map(|(width, height)| (u64::from(width), u64::from(height)))
        .unwrap_or((0, 0))
}

pub fn capture_window_snapshot_bgra(hwnd: HWND) -> Result<(u32, u32, Vec<u8>), String> {
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return Err("Window is minimized".to_owned());
    }

    let Some(rect) = get_window_physical_rect(hwnd) else {
        return Err("Window has zero dimensions".to_owned());
    };
    let Some((width, height)) = rect_dimensions(&rect) else {
        return Err("Window has zero dimensions".to_owned());
    };
    let width_i32 = i32::try_from(width).map_err(|_| "Window width is too large".to_owned())?;
    let height_i32 = i32::try_from(height).map_err(|_| "Window height is too large".to_owned())?;

    unsafe {
        let window_dc = GetDC(Some(hwnd));
        if window_dc.is_invalid() {
            return Err("GetDC failed for window".to_owned());
        }
        let mem_dc = CreateCompatibleDC(Some(window_dc));
        let bitmap = CreateCompatibleBitmap(window_dc, width_i32, height_i32);
        let old = SelectObject(mem_dc, bitmap.into());

        let rendered = PrintWindow(hwnd, mem_dc, PRINT_WINDOW_FLAGS(2)).as_bool();
        if !rendered {
            let _ = BitBlt(
                mem_dc,
                0,
                0,
                width_i32,
                height_i32,
                Some(window_dc),
                0,
                0,
                SRCCOPY,
            );
        }

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width_i32;
        bmi.bmiHeader.biHeight = -height_i32;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;

        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        let copied = GetDIBits(
            mem_dc,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(Some(hwnd), window_dc);

        if copied == 0 {
            return Err("GetDIBits failed for window snapshot".to_owned());
        }

        for alpha in pixels[3..].iter_mut().step_by(4) {
            *alpha = 255;
        }

        Ok((width, height, pixels))
    }
}
