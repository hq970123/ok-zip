//! The Bandizip-style command band: a blue gradient strip with large white
//! line icons (Lucide) and labels, split buttons with a drop-down arrow, and
//! right-aligned buttons. Owner-drawn child window.
//!
//! Notifications to the parent:
//! * `WM_COMMAND(cmd, band_hwnd)` when a button is clicked;
//! * `WM_BAND_DROPDOWN(cmd, &RECT)` (sent) when a drop-down arrow is pressed;
//!   the RECT is the button in screen coordinates.

use super::i18n::{tr, S};
use super::theme::*;
use super::util::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_BAND_DROPDOWN: u32 = WM_APP + 20;
const CLASS: &str = "OKZipBand";

#[derive(Clone, Copy)]
pub struct Btn {
    pub cmd: i32,
    pub icon: &'static str,
    pub label: S,
    pub dropdown: bool,
    pub right: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Hit {
    idx: usize,
    arrow: bool,
}

struct Band {
    hwnd: HWND,
    btns: Vec<Btn>,
    rects: RefCell<Vec<(RECT, i32)>>, // button rect, x where the arrow part starts (or right edge)
    enabled: RefCell<HashMap<i32, bool>>,
    font: Cell<HFONT>,
    dpi: Cell<u32>,
    hot: Cell<Option<Hit>>,
    pressed: Cell<Option<Hit>>,
    tracking: Cell<bool>,
}

thread_local! {
    static BANDS: RefCell<HashMap<isize, &'static Band>> = RefCell::new(HashMap::new());
}

fn band(hwnd: HWND) -> Option<&'static Band> {
    BANDS.with(|b| b.borrow().get(&(hwnd as isize)).copied())
}

pub fn create(parent: HWND, btns: Vec<Btn>) -> HWND {
    unsafe {
        let class = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(band_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance(),
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), null(), WS_CHILD | WS_VISIBLE, 0, 0, 0, 0, parent, null_mut(), hinstance(), null());
        let b: &'static Band = Box::leak(Box::new(Band {
            hwnd,
            btns,
            rects: RefCell::new(vec![]),
            enabled: RefCell::new(HashMap::new()),
            font: Cell::new(null_mut()),
            dpi: Cell::new(dpi_of(parent)),
            hot: Cell::new(None),
            pressed: Cell::new(None),
            tracking: Cell::new(false),
        }));
        BANDS.with(|m| m.borrow_mut().insert(hwnd as isize, b));
        hwnd
    }
}

/// Height of the band at `dpi`.
pub fn height(dpi: u32) -> i32 {
    scale(78, dpi)
}

pub fn set_font(hwnd: HWND, font: HFONT, dpi: u32) {
    if let Some(b) = band(hwnd) {
        b.font.set(font);
        b.dpi.set(dpi);
        b.layout();
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

pub fn enable(hwnd: HWND, cmd: i32, on: bool) {
    if let Some(b) = band(hwnd) {
        let changed = b.enabled.borrow_mut().insert(cmd, on) != Some(on);
        if changed {
            unsafe { InvalidateRect(hwnd, null(), 0) };
        }
    }
}

impl Band {
    fn is_enabled(&self, cmd: i32) -> bool {
        self.enabled.borrow().get(&cmd).copied().unwrap_or(true)
    }

    fn layout(&self) {
        let dpi = self.dpi.get();
        let mut rc: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetClientRect(self.hwnd, &mut rc) };
        let h = rc.bottom - 1;
        let hdc = unsafe { GetDC(self.hwnd) };
        let widths: Vec<(i32, i32)> = self
            .btns
            .iter()
            .map(|b| {
                let main = (text_width(hdc, tr(b.label), self.font.get()) + scale(22, dpi)).max(scale(62, dpi));
                (main, if b.dropdown { scale(16, dpi) } else { 0 })
            })
            .collect();
        unsafe { ReleaseDC(self.hwnd, hdc) };
        let mut out = vec![(RECT { left: 0, top: 0, right: 0, bottom: 0 }, 0); self.btns.len()];
        let mut x = scale(8, dpi);
        for (i, b) in self.btns.iter().enumerate().filter(|(_, b)| !b.right) {
            let (m, a) = widths[i];
            out[i] = (RECT { left: x, top: 0, right: x + m + a, bottom: h }, x + m);
            x += m + a + scale(if b.dropdown { 6 } else { 2 }, dpi);
        }
        let mut x = rc.right - scale(8, dpi);
        for (i, _) in self.btns.iter().enumerate().rev().filter(|(_, b)| b.right) {
            let (m, a) = widths[i];
            out[i] = (RECT { left: x - m - a, top: 0, right: x, bottom: h }, x - a);
            x -= m + a + scale(2, dpi);
        }
        *self.rects.borrow_mut() = out;
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.rects.borrow().iter().enumerate().find_map(|(i, (r, split))| {
            (x >= r.left && x < r.right && y >= r.top && y < r.bottom).then(|| Hit { idx: i, arrow: self.btns[i].dropdown && x >= *split })
        })
    }

    fn paint(&self, hdc: HDC) {
        let dpi = self.dpi.get();
        let mut rc: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetClientRect(self.hwnd, &mut rc) };
        let buf = Buffered::new(hdc, rc.right, rc.bottom);
        let dc = buf.dc;
        gradient_v(dc, &rc, BAND_TOP, BAND_BOTTOM);
        fill(dc, &RECT { left: 0, top: rc.bottom - 1, right: rc.right, bottom: rc.bottom }, BAND_LINE);
        let icon = scale(32, dpi);
        let rects = self.rects.borrow();
        for (i, b) in self.btns.iter().enumerate() {
            let (r, split) = rects[i];
            let on = self.is_enabled(b.cmd);
            let hot = self.hot.get().filter(|h| h.idx == i && on);
            let pressed = self.pressed.get().filter(|h| h.idx == i && on);
            if let Some(h) = hot.or(pressed) {
                let inner = RECT { left: r.left, top: r.top + scale(4, dpi), right: r.right, bottom: r.bottom - scale(4, dpi) };
                fill_alpha(dc, &inner, WHITE, 34);
                if b.dropdown {
                    let part = if h.arrow { RECT { left: split, ..inner } } else { RECT { right: split, ..inner } };
                    if let Some(p) = pressed {
                        if p.arrow == h.arrow {
                            fill_alpha(dc, &part, WHITE, 40);
                        }
                    }
                    fill_alpha(dc, &RECT { left: split, right: split + 1, top: inner.top + scale(6, dpi), bottom: inner.bottom - scale(6, dpi) }, WHITE, 90);
                } else if pressed.is_some() {
                    fill_alpha(dc, &inner, WHITE, 40);
                }
            }
            let alpha = if on { 255 } else { 95 };
            let ix = r.left + (split - r.left - icon) / 2;
            let iy = scale(9, dpi);
            draw_icon(dc, b.icon, ix, iy, icon, WHITE, alpha);
            if b.dropdown {
                let c = scale(12, dpi);
                draw_icon(dc, "chevron", split + (r.right - split - c) / 2, iy + (icon - c) / 2, c, WHITE, alpha);
            }
            let label = RECT { left: r.left, top: iy + icon + scale(5, dpi), right: split, bottom: r.bottom - scale(4, dpi) };
            text(dc, tr(b.label), &label, self.font.get(), if on { WHITE } else { TEXT_DISABLED }, DT_CENTER | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);
        }
    }

    fn set_hot(&self, h: Option<Hit>) {
        if self.hot.get() != h {
            self.hot.set(h);
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }
    }
}

fn pt(lp: LPARAM) -> (i32, i32) {
    ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32)
}

unsafe extern "system" fn band_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(b) = band(hwnd) else { return DefWindowProcW(hwnd, msg, wp, lp) };
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                b.paint(hdc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_SIZE => {
                b.layout();
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                if !b.tracking.get() {
                    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut t);
                    b.tracking.set(true);
                }
                let (x, y) = pt(lp);
                b.set_hot(b.hit(x, y));
                0
            }
            WM_MOUSELEAVE => {
                b.tracking.set(false);
                b.set_hot(None);
                0
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = pt(lp);
                let Some(h) = b.hit(x, y) else { return 0 };
                let btn = b.btns[h.idx];
                if !b.is_enabled(btn.cmd) {
                    return 0;
                }
                b.pressed.set(Some(h));
                InvalidateRect(hwnd, null(), 0);
                UpdateWindow(hwnd);
                if h.arrow {
                    let (r, _) = b.rects.borrow()[h.idx];
                    let mut sr = r;
                    MapWindowPoints(hwnd, null_mut(), &mut sr as *mut RECT as *mut POINT, 2);
                    SendMessageW(GetParent(hwnd), WM_BAND_DROPDOWN, btn.cmd as usize, &sr as *const RECT as isize);
                    b.pressed.set(None);
                    b.set_hot(None);
                    InvalidateRect(hwnd, null(), 0);
                } else {
                    SetCapture(hwnd);
                }
                0
            }
            WM_LBUTTONUP => {
                let pressed = b.pressed.take();
                if GetCapture() == hwnd {
                    ReleaseCapture();
                }
                InvalidateRect(hwnd, null(), 0);
                let (x, y) = pt(lp);
                if let (Some(p), Some(h)) = (pressed, b.hit(x, y)) {
                    if p == h && !p.arrow {
                        let cmd = b.btns[p.idx].cmd;
                        PostMessageW(GetParent(hwnd), WM_COMMAND, cmd as usize, hwnd as isize);
                    }
                }
                0
            }
            WM_CAPTURECHANGED => {
                b.pressed.set(None);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
