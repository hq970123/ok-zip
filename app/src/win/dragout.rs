//! Drag-out: a minimal hand-written `IDataObject` (CF_HDROP only) and
//! `IDropSource`, so files extracted to a temp folder can be dragged from the
//! main window onto Explorer or the desktop with `DoDragDrop`.
//!
//! windows-sys has no COM implementation helpers, so the vtables are spelled
//! out by hand. Both objects live on the UI thread only (STA), so a plain
//! `Cell` reference count is enough.

use super::util::wide_path;
use std::cell::Cell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use windows_sys::core::{BOOL, GUID, HRESULT};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Com::FORMATETC;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows_sys::Win32::System::Ole::{DoDragDrop, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY};
use windows_sys::Win32::UI::Shell::{SHCreateStdEnumFmtEtc, DROPFILES};

const S_OK: HRESULT = 0;
const DRAGDROP_S_DROP: HRESULT = 0x0004_0100;
const DRAGDROP_S_CANCEL: HRESULT = 0x0004_0101;
const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = 0x0004_0102;
const DV_E_FORMATETC: HRESULT = 0x8004_0064_u32 as i32;
const OLE_E_ADVISENOTSUPPORTED: HRESULT = 0x8004_0003_u32 as i32;
const E_NOTIMPL: HRESULT = 0x8000_4001_u32 as i32;
const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as i32;
const E_POINTER: HRESULT = 0x8000_4003_u32 as i32;
const E_OUTOFMEMORY: HRESULT = 0x8007_000E_u32 as i32;
const TYMED_HGLOBAL: u32 = 1;
const DVASPECT_CONTENT: u32 = 1;
const DATADIR_GET: u32 = 1;
const MK_LBUTTON: u32 = 0x0001;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const IID_IDROPSOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_c000_000000000046);

/// STGMEDIUM laid out by hand (the windows-sys one needs extra features).
#[repr(C)]
struct StgMedium {
    tymed: u32,
    hglobal: *mut c_void,
    unk_for_release: *mut c_void,
}

fn same_guid(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

// ------------------------------------------------------------- IDataObject

#[repr(C)]
struct DataObjectVtbl {
    query_interface: unsafe extern "system" fn(*mut DataObject, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DataObject) -> u32,
    release: unsafe extern "system" fn(*mut DataObject) -> u32,
    get_data: unsafe extern "system" fn(*mut DataObject, *const FORMATETC, *mut StgMedium) -> HRESULT,
    get_data_here: unsafe extern "system" fn(*mut DataObject, *const FORMATETC, *mut StgMedium) -> HRESULT,
    query_get_data: unsafe extern "system" fn(*mut DataObject, *const FORMATETC) -> HRESULT,
    get_canonical_format_etc: unsafe extern "system" fn(*mut DataObject, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    set_data: unsafe extern "system" fn(*mut DataObject, *const FORMATETC, *const StgMedium, BOOL) -> HRESULT,
    enum_format_etc: unsafe extern "system" fn(*mut DataObject, u32, *mut *mut c_void) -> HRESULT,
    d_advise: unsafe extern "system" fn(*mut DataObject, *const FORMATETC, u32, *mut c_void, *mut u32) -> HRESULT,
    d_unadvise: unsafe extern "system" fn(*mut DataObject, u32) -> HRESULT,
    enum_d_advise: unsafe extern "system" fn(*mut DataObject, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DataObject {
    vtbl: *const DataObjectVtbl,
    refs: Cell<u32>,
    /// DROPFILES header + double-NUL-terminated UTF-16 path list.
    hdrop: Vec<u8>,
}

fn hdrop_format() -> FORMATETC {
    FORMATETC { cfFormat: CF_HDROP, ptd: null_mut(), dwAspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL }
}

fn wants_hdrop(f: *const FORMATETC) -> bool {
    if f.is_null() {
        return false;
    }
    let f = unsafe { &*f };
    f.cfFormat == CF_HDROP && f.tymed & TYMED_HGLOBAL != 0 && f.dwAspect == DVASPECT_CONTENT
}

unsafe extern "system" fn do_qi(this: *mut DataObject, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe {
        if out.is_null() {
            return E_POINTER;
        }
        if same_guid(&*riid, &IID_IUNKNOWN) || same_guid(&*riid, &IID_IDATAOBJECT) {
            do_add_ref(this);
            *out = this as *mut c_void;
            S_OK
        } else {
            *out = null_mut();
            E_NOINTERFACE
        }
    }
}
unsafe extern "system" fn do_add_ref(this: *mut DataObject) -> u32 {
    let o = unsafe { &*this };
    o.refs.set(o.refs.get() + 1);
    o.refs.get()
}
unsafe extern "system" fn do_release(this: *mut DataObject) -> u32 {
    let n = {
        let o = unsafe { &*this };
        o.refs.set(o.refs.get() - 1);
        o.refs.get()
    };
    if n == 0 {
        drop(unsafe { Box::from_raw(this) });
    }
    n
}
unsafe extern "system" fn do_get_data(this: *mut DataObject, f: *const FORMATETC, medium: *mut StgMedium) -> HRESULT {
    if !wants_hdrop(f) || medium.is_null() {
        return DV_E_FORMATETC;
    }
    unsafe {
        let data = &(*this).hdrop;
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, data.len());
        if h.is_null() {
            return E_OUTOFMEMORY;
        }
        let p = GlobalLock(h) as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        *medium = StgMedium { tymed: TYMED_HGLOBAL, hglobal: h, unk_for_release: null_mut() };
    }
    S_OK
}
unsafe extern "system" fn do_get_data_here(_: *mut DataObject, _: *const FORMATETC, _: *mut StgMedium) -> HRESULT {
    E_NOTIMPL
}
unsafe extern "system" fn do_query_get_data(_: *mut DataObject, f: *const FORMATETC) -> HRESULT {
    if wants_hdrop(f) {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}
unsafe extern "system" fn do_canonical(_: *mut DataObject, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
    if !out.is_null() {
        unsafe { (*out).ptd = null_mut() };
    }
    E_NOTIMPL
}
unsafe extern "system" fn do_set_data(_: *mut DataObject, _: *const FORMATETC, _: *const StgMedium, _: BOOL) -> HRESULT {
    // Explorer offers drag-image / drop-description formats here; not needed.
    E_NOTIMPL
}
unsafe extern "system" fn do_enum(_: *mut DataObject, dir: u32, out: *mut *mut c_void) -> HRESULT {
    if dir != DATADIR_GET {
        return E_NOTIMPL;
    }
    let fmt = hdrop_format();
    unsafe { SHCreateStdEnumFmtEtc(1, &fmt, out) }
}
unsafe extern "system" fn do_dadvise(_: *mut DataObject, _: *const FORMATETC, _: u32, _: *mut c_void, _: *mut u32) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}
unsafe extern "system" fn do_dunadvise(_: *mut DataObject, _: u32) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}
unsafe extern "system" fn do_enum_dadvise(_: *mut DataObject, _: *mut *mut c_void) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

static DATA_OBJECT_VTBL: DataObjectVtbl = DataObjectVtbl {
    query_interface: do_qi,
    add_ref: do_add_ref,
    release: do_release,
    get_data: do_get_data,
    get_data_here: do_get_data_here,
    query_get_data: do_query_get_data,
    get_canonical_format_etc: do_canonical,
    set_data: do_set_data,
    enum_format_etc: do_enum,
    d_advise: do_dadvise,
    d_unadvise: do_dunadvise,
    enum_d_advise: do_enum_dadvise,
};

/// DROPFILES block for `paths` (wide chars, double-NUL terminated).
fn build_hdrop(paths: &[PathBuf]) -> Vec<u8> {
    let header = std::mem::size_of::<DROPFILES>();
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        list.extend(wide_path(p)); // includes the NUL
    }
    list.push(0);
    let mut out = vec![0u8; header + list.len() * 2];
    let df = DROPFILES { pFiles: header as u32, pt: POINT { x: 0, y: 0 }, fNC: 0, fWide: 1 };
    unsafe {
        std::ptr::copy_nonoverlapping(&df as *const DROPFILES as *const u8, out.as_mut_ptr(), header);
        std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, out.as_mut_ptr().add(header), list.len() * 2);
    }
    out
}

// ------------------------------------------------------------- IDropSource

#[repr(C)]
struct DropSourceVtbl {
    query_interface: unsafe extern "system" fn(*mut DropSource, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DropSource) -> u32,
    release: unsafe extern "system" fn(*mut DropSource) -> u32,
    query_continue_drag: unsafe extern "system" fn(*mut DropSource, BOOL, u32) -> HRESULT,
    give_feedback: unsafe extern "system" fn(*mut DropSource, DROPEFFECT) -> HRESULT,
}

#[repr(C)]
struct DropSource {
    vtbl: *const DropSourceVtbl,
    refs: Cell<u32>,
}

unsafe extern "system" fn ds_qi(this: *mut DropSource, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe {
        if out.is_null() {
            return E_POINTER;
        }
        if same_guid(&*riid, &IID_IUNKNOWN) || same_guid(&*riid, &IID_IDROPSOURCE) {
            ds_add_ref(this);
            *out = this as *mut c_void;
            S_OK
        } else {
            *out = null_mut();
            E_NOINTERFACE
        }
    }
}
unsafe extern "system" fn ds_add_ref(this: *mut DropSource) -> u32 {
    let o = unsafe { &*this };
    o.refs.set(o.refs.get() + 1);
    o.refs.get()
}
unsafe extern "system" fn ds_release(this: *mut DropSource) -> u32 {
    let n = {
        let o = unsafe { &*this };
        o.refs.set(o.refs.get() - 1);
        o.refs.get()
    };
    if n == 0 {
        drop(unsafe { Box::from_raw(this) });
    }
    n
}
unsafe extern "system" fn ds_query_continue(_: *mut DropSource, escape: BOOL, keys: u32) -> HRESULT {
    if escape != 0 {
        DRAGDROP_S_CANCEL
    } else if keys & MK_LBUTTON == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}
unsafe extern "system" fn ds_feedback(_: *mut DropSource, _: DROPEFFECT) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

static DROP_SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
    query_interface: ds_qi,
    add_ref: ds_add_ref,
    release: ds_release,
    query_continue_drag: ds_query_continue,
    give_feedback: ds_feedback,
};

/// Run a modal OLE drag of `paths` (copy only). Returns true if dropped.
/// Requires OleInitialize on the calling (UI) thread.
pub fn drag_files(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let data = Box::into_raw(Box::new(DataObject { vtbl: &DATA_OBJECT_VTBL, refs: Cell::new(1), hdrop: build_hdrop(paths) }));
    let source = Box::into_raw(Box::new(DropSource { vtbl: &DROP_SOURCE_VTBL, refs: Cell::new(1) }));
    let mut effect: DROPEFFECT = 0;
    let hr = unsafe { DoDragDrop(data as *mut c_void, source as *mut c_void, DROPEFFECT_COPY, &mut effect) };
    unsafe {
        do_release(data);
        ds_release(source);
    }
    hr == DRAGDROP_S_DROP && effect != 0
}
