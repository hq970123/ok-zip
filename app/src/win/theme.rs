//! Drawing helpers for the custom-painted parts of the UI (the blue command
//! band and the start page): Lucide icon masks tinted at runtime, gradients
//! and translucent fills. Plain GDI + AlphaBlend, no extra dependencies.

use super::icons_gen::{MASKS, TABLE};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

// Palette (close to Bandizip's blue).
pub const BAND_TOP: COLORREF = rgb(0x47, 0x9c, 0xee);
pub const BAND_BOTTOM: COLORREF = rgb(0x2b, 0x7c, 0xda);
pub const BAND_LINE: COLORREF = rgb(0x22, 0x6b, 0xc2);
pub const HOME_TOP: COLORREF = rgb(0x4f, 0xa1, 0xee);
pub const HOME_BOTTOM: COLORREF = rgb(0x1c, 0x6a, 0xcd);
pub const WHITE: COLORREF = rgb(0xff, 0xff, 0xff);
pub const TEXT_DISABLED: COLORREF = rgb(0xa9, 0xcb, 0xf3);
pub const SPLITTER: COLORREF = rgb(0xe6, 0xe9, 0xee);
pub const TILE_BORDER: COLORREF = rgb(0xc3, 0xd6, 0xec);
pub const TILE_HOT: COLORREF = rgb(0xe9, 0xf3, 0xfe);
pub const TILE_ICON: COLORREF = rgb(0x45, 0x4b, 0x55);
pub const TILE_TEXT: COLORREF = rgb(0x1f, 0x23, 0x29);
pub const HINT_TEXT: COLORREF = rgb(0xdd, 0xec, 0xfc);

fn best_mask(name: &str, px: i32) -> Option<(i32, &'static [u8])> {
    let mut sizes: Vec<(i32, usize)> = TABLE.iter().filter(|(n, _, _)| *n == name).map(|(_, s, off)| (*s as i32, *off)).collect();
    sizes.sort();
    let (s, off) = sizes.iter().copied().find(|(s, _)| *s >= px).or_else(|| sizes.last().copied())?;
    Some((s, &MASKS[off..off + (s * s) as usize]))
}

type Key = (&'static str, i32, COLORREF);
thread_local! {
    static CACHE: RefCell<HashMap<Key, (HBITMAP, i32)>> = RefCell::new(HashMap::new());
}

/// A 32-bpp top-down DIB section; returns the bitmap and its pixel buffer.
unsafe fn dib(w: i32, h: i32) -> (HBITMAP, *mut u32) {
    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = null_mut();
        let bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        (bmp, bits as *mut u32)
    }
}

fn tinted(name: &'static str, px: i32, color: COLORREF) -> Option<(HBITMAP, i32)> {
    let key = (name, px, color);
    if let Some(v) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return Some(v);
    }
    let (s, mask) = best_mask(name, px)?;
    let (r, g, b) = (color & 0xff, (color >> 8) & 0xff, (color >> 16) & 0xff);
    unsafe {
        let (bmp, bits) = dib(s, s);
        if bmp.is_null() || bits.is_null() {
            return None;
        }
        let px_buf = std::slice::from_raw_parts_mut(bits, (s * s) as usize);
        for (dst, &a) in px_buf.iter_mut().zip(mask) {
            let a = a as u32;
            // premultiplied BGRA
            *dst = (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255);
        }
        CACHE.with(|c| c.borrow_mut().insert(key, (bmp, s)));
        Some((bmp, s))
    }
}

fn blend(alpha: u8, per_pixel: bool) -> BLENDFUNCTION {
    BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: if per_pixel { AC_SRC_ALPHA as u8 } else { 0 } }
}

/// Draw the named Lucide icon at `px` x `px`, tinted with `color`, at the
/// given overall opacity.
pub fn draw_icon(hdc: HDC, name: &'static str, x: i32, y: i32, px: i32, color: COLORREF, alpha: u8) {
    let Some((bmp, s)) = tinted(name, px, color) else { return };
    unsafe {
        let mem = CreateCompatibleDC(hdc);
        let old = SelectObject(mem, bmp as HGDIOBJ);
        AlphaBlend(hdc, x, y, px, px, mem, 0, 0, s, s, blend(alpha, true));
        SelectObject(mem, old);
        DeleteDC(mem);
    }
}

pub fn gradient_v(hdc: HDC, rc: &RECT, top: COLORREF, bottom: COLORREF) {
    let v = |x: i32, y: i32, c: COLORREF| TRIVERTEX {
        x,
        y,
        Red: ((c & 0xff) as u16) << 8,
        Green: (((c >> 8) & 0xff) as u16) << 8,
        Blue: (((c >> 16) & 0xff) as u16) << 8,
        Alpha: 0xff00,
    };
    let verts = [v(rc.left, rc.top, top), v(rc.right, rc.bottom, bottom)];
    let mesh = GRADIENT_RECT { UpperLeft: 0, LowerRight: 1 };
    unsafe {
        GradientFill(hdc, verts.as_ptr(), 2, &mesh as *const _ as *const core::ffi::c_void, 1, GRADIENT_FILL_RECT_V);
    }
}

pub fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(color);
        FillRect(hdc, rc, b);
        DeleteObject(b as HGDIOBJ);
    }
}

pub fn frame(hdc: HDC, rc: &RECT, color: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(color);
        FrameRect(hdc, rc, b);
        DeleteObject(b as HGDIOBJ);
    }
}

/// Fill `rc` with `color` at opacity `alpha` (0-255).
pub fn fill_alpha(hdc: HDC, rc: &RECT, color: COLORREF, alpha: u8) {
    unsafe {
        let (bmp, bits) = dib(1, 1);
        if bmp.is_null() {
            return;
        }
        *bits = 0xff00_0000 | ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff);
        let mem = CreateCompatibleDC(hdc);
        let old = SelectObject(mem, bmp as HGDIOBJ);
        AlphaBlend(hdc, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, mem, 0, 0, 1, 1, blend(alpha, false));
        SelectObject(mem, old);
        DeleteDC(mem);
        DeleteObject(bmp as HGDIOBJ);
    }
}

/// Draw `text` in `rc` with the given font, color and DT_* flags.
pub fn text(hdc: HDC, s: &str, rc: &RECT, font: HFONT, color: COLORREF, flags: u32) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = *rc;
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        SetBkMode(hdc, TRANSPARENT as _);
        SetTextColor(hdc, color);
        DrawTextW(hdc, w.as_mut_ptr(), w.len() as i32, &mut r, flags as _);
        SelectObject(hdc, old);
    }
}

pub fn text_width(hdc: HDC, s: &str, font: HFONT) -> i32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        let mut sz: SIZE = std::mem::zeroed();
        GetTextExtentPoint32W(hdc, w.as_ptr(), w.len() as i32, &mut sz);
        SelectObject(hdc, old);
        sz.cx
    }
}

/// Off-screen buffer for flicker-free WM_PAINT.
pub struct Buffered {
    target: HDC,
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
}

impl Buffered {
    pub fn new(target: HDC, w: i32, h: i32) -> Self {
        unsafe {
            let dc = CreateCompatibleDC(target);
            let bmp = CreateCompatibleBitmap(target, w.max(1), h.max(1));
            let old = SelectObject(dc, bmp as HGDIOBJ);
            Buffered { target, dc, bmp, old, w, h }
        }
    }
}

impl Drop for Buffered {
    fn drop(&mut self) {
        unsafe {
            BitBlt(self.target, 0, 0, self.w, self.h, self.dc, 0, 0, SRCCOPY);
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}
