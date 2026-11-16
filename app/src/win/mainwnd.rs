//! The Bandizip-style main window: menu bar, big-button toolbar, folder tree,
//! virtual report list view with shell icons, status bar, splitter, drag & drop.

use super::dialogs::{create_dialog, extract_dialog, settings_dialog};
use super::i18n::{tr, trf, S};
use super::ids::*;
use super::progress::{conflict_prompt, report_error, run_task, with_password};
use super::util::*;
use okzip_core::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS_NAME: &str = "OKZipMainWindow";
const SPLIT_GAP: i32 = 5;

// ---------------------------------------------------------------- model

struct Folder {
    path: String,
    name: String,
    parent: Option<usize>,
    subs: Vec<usize>,
    files: Vec<usize>,
    size: u64,
    packed: u64,
    mtime: Option<i64>,
    htree: Cell<isize>,
}

struct Model {
    info: ArchiveInfo,
    folders: Vec<Folder>,
}

impl Model {
    fn build(info: ArchiveInfo) -> Model {
        let mut m = Model {
            info,
            folders: vec![Folder {
                path: String::new(),
                name: String::new(),
                parent: None,
                subs: vec![],
                files: vec![],
                size: 0,
                packed: 0,
                mtime: None,
                htree: Cell::new(0),
            }],
        };
        let mut index: HashMap<String, usize> = HashMap::new();
        index.insert(String::new(), 0);
        fn ensure(m: &mut Model, index: &mut HashMap<String, usize>, path: &str) -> usize {
            if let Some(&i) = index.get(path) {
                return i;
            }
            let (parent_path, name) = match path.rfind('/') {
                Some(i) => (&path[..i], &path[i + 1..]),
                None => ("", path),
            };
            let p = ensure(m, index, parent_path);
            let id = m.folders.len();
            m.folders.push(Folder {
                path: path.to_string(),
                name: name.to_string(),
                parent: Some(p),
                subs: vec![],
                files: vec![],
                size: 0,
                packed: 0,
                mtime: None,
                htree: Cell::new(0),
            });
            m.folders[p].subs.push(id);
            index.insert(path.to_string(), id);
            id
        }
        let n = m.info.entries.len();
        for i in 0..n {
            let (is_dir, path, parent, size, packed, mtime) = {
                let e = &m.info.entries[i];
                (e.is_dir, e.path.clone(), e.parent().to_string(), e.size, e.packed_size.unwrap_or(0), e.modified)
            };
            if is_dir {
                let f = ensure(&mut m, &mut index, &path);
                m.folders[f].mtime = mtime;
            } else {
                let mut f = ensure(&mut m, &mut index, &parent);
                m.folders[f].files.push(i);
                loop {
                    m.folders[f].size += size;
                    m.folders[f].packed += packed;
                    match m.folders[f].parent {
                        Some(p) => f = p,
                        None => break,
                    }
                }
            }
        }
        let names: Vec<String> = m.folders.iter().map(|f| f.name.to_lowercase()).collect();
        for f in &mut m.folders {
            f.subs.sort_by(|a, b| names[*a].cmp(&names[*b]));
        }
        m
    }

    fn folder_by_path(&self, path: &str) -> Option<usize> {
        self.folders.iter().position(|f| f.path == path)
    }

    fn writable(&self) -> bool {
        self.info.format.is_writable() && self.info.volumes <= 1 && volume_set(&self.info.path).is_none()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Up,
    Folder(usize),
    File(usize),
}

// ---------------------------------------------------------------- app state

pub struct App {
    hwnd: Cell<HWND>,
    toolbar: Cell<HWND>,
    tree: Cell<HWND>,
    list: Cell<HWND>,
    status: Cell<HWND>,
    menu: Cell<HMENU>,
    font: Cell<HFONT>,
    dpi: Cell<u32>,
    split: Cell<i32>,
    dragging: Cell<bool>,
    show_tree: Cell<bool>,
    model: RefCell<Option<Model>>,
    password: RefCell<Option<String>>,
    cur: Cell<usize>,
    rows: RefCell<Vec<Row>>,
    sort: Cell<(i32, bool)>,
    temp_root: PathBuf,
    temp_counter: Cell<u32>,
    toolbar_images: Cell<HIMAGELIST>,
}

thread_local! {
    static APP: Cell<Option<&'static App>> = const { Cell::new(None) };
}

fn app() -> Option<&'static App> {
    APP.with(|a| a.get())
}

// ---------------------------------------------------------------- creation

pub fn create_main_window(initial: Option<PathBuf>) -> HWND {
    unsafe {
        let hinst = hinstance();
        let class = wide(CLASS_NAME);
        let big = GetSystemMetrics(SM_CXICON);
        let small = GetSystemMetrics(SM_CXSMICON);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: LoadImageW(hinst, make_int_resource(IDI_APP), IMAGE_ICON, big, big, LR_DEFAULTCOLOR) as HICON,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: LoadImageW(hinst, make_int_resource(IDI_APP), IMAGE_ICON, small, small, LR_DEFAULTCOLOR) as HICON,
        };
        RegisterClassExW(&wc);

        let state: &'static App = Box::leak(Box::new(App {
            hwnd: Cell::new(null_mut()),
            toolbar: Cell::new(null_mut()),
            tree: Cell::new(null_mut()),
            list: Cell::new(null_mut()),
            status: Cell::new(null_mut()),
            menu: Cell::new(null_mut()),
            font: Cell::new(null_mut()),
            dpi: Cell::new(96),
            split: Cell::new(230),
            dragging: Cell::new(false),
            show_tree: Cell::new(true),
            model: RefCell::new(None),
            password: RefCell::new(None),
            cur: Cell::new(0),
            rows: RefCell::new(vec![]),
            sort: Cell::new((0, true)),
            temp_root: std::env::temp_dir().join(format!("OKZip-{}", std::process::id())),
            temp_counter: Cell::new(0),
            toolbar_images: Cell::new(0),
        }));
        APP.with(|a| a.set(Some(state)));

        let sys_dpi = windows_sys::Win32::UI::HiDpi::GetDpiForSystem().max(96);
        let title = wide(tr(S::AppName));
        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(980, sys_dpi),
            scale(640, sys_dpi),
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        ShowWindow(hwnd, SW_SHOWNORMAL);
        if let Some(path) = initial {
            state.open_archive(&path);
        }
        hwnd
    }
}

/// Keyboard accelerators for the main window.
pub fn accelerators() -> HACCEL {
    let ctrl = FVIRTKEY | FCONTROL;
    let v = FVIRTKEY;
    let table = [
        ACCEL { fVirt: ctrl, key: b'O' as u16, cmd: ID_OPEN as u16 },
        ACCEL { fVirt: ctrl, key: b'N' as u16, cmd: ID_NEW as u16 },
        ACCEL { fVirt: ctrl, key: b'E' as u16, cmd: ID_EXTRACT as u16 },
        ACCEL { fVirt: ctrl, key: b'T' as u16, cmd: ID_TEST as u16 },
        ACCEL { fVirt: ctrl, key: b'A' as u16, cmd: ID_SELECT_ALL as u16 },
        ACCEL { fVirt: v, key: VK_DELETE, cmd: ID_DELETE as u16 },
        ACCEL { fVirt: v, key: VK_F5, cmd: ID_REFRESH as u16 },
        ACCEL { fVirt: v, key: VK_BACK, cmd: ID_UP as u16 },
        ACCEL { fVirt: v, key: VK_RETURN, cmd: ID_VIEW as u16 },
    ];
    unsafe { CreateAcceleratorTableW(table.as_ptr(), table.len() as i32) }
}

impl App {
    fn on_create(&self, hwnd: HWND) {
        unsafe {
            self.hwnd.set(hwnd);
            self.dpi.set(dpi_of(hwnd));
            let hinst = hinstance();
            self.menu.set(build_menu());
            SetMenu(hwnd, self.menu.get());

            // Toolbar
            let tb = CreateWindowExW(
                0,
                TOOLBARCLASSNAMEW,
                null(),
                WS_CHILD | WS_VISIBLE | TBSTYLE_FLAT | TBSTYLE_TOOLTIPS | CCS_TOP as u32 | CCS_NODIVIDER as u32,
                0,
                0,
                0,
                0,
                hwnd,
                null_mut(),
                hinst,
                null(),
            );
            self.toolbar.set(tb);
            SendMessageW(tb, TB_BUTTONSTRUCTSIZE, std::mem::size_of::<TBBUTTON>(), 0);
            SendMessageW(tb, TB_SETEXTENDEDSTYLE, 0, TBSTYLE_EX_DOUBLEBUFFER as isize);
            self.build_toolbar();

            // Tree
            let tree = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                WC_TREEVIEWW,
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | TVS_HASBUTTONS | TVS_HASLINES | TVS_LINESATROOT | TVS_SHOWSELALWAYS,
                0,
                0,
                0,
                0,
                hwnd,
                null_mut(),
                hinst,
                null(),
            );
            self.tree.set(tree);
            let explorer = wide("Explorer");
            SetWindowTheme(tree, explorer.as_ptr(), null());
            SendMessageW(tree, TVM_SETEXTENDEDSTYLE, TVS_EX_DOUBLEBUFFER as usize, TVS_EX_DOUBLEBUFFER as isize);
            SendMessageW(tree, TVM_SETIMAGELIST, TVSIL_NORMAL as usize, system_image_list());

            // List
            let list = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                WC_LISTVIEWW,
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS,
                0,
                0,
                0,
                0,
                hwnd,
                null_mut(),
                hinst,
                null(),
            );
            self.list.set(list);
            SetWindowTheme(list, explorer.as_ptr(), null());
            let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_HEADERDRAGDROP | LVS_EX_LABELTIP;
            SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, ex as usize, ex as isize);
            SendMessageW(list, LVM_SETIMAGELIST, LVSIL_SMALL as usize, system_image_list());
            self.add_columns();

            // Status bar
            let status = CreateWindowExW(0, STATUSCLASSNAMEW, null(), WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP, 0, 0, 0, 0, hwnd, null_mut(), hinst, null());
            self.status.set(status);

            self.apply_font();
            DragAcceptFiles(hwnd, 1);
            self.update_ui();
            self.update_status();
        }
    }

    fn apply_font(&self) {
        let old = self.font.get();
        let f = create_ui_font(self.dpi.get());
        self.font.set(f);
        for h in [self.toolbar.get(), self.tree.get(), self.list.get(), self.status.get()] {
            set_font(h, f);
        }
        if !old.is_null() {
            unsafe { DeleteObject(old as HGDIOBJ) };
        }
    }

    fn build_toolbar(&self) {
        unsafe {
            let tb = self.toolbar.get();
            let size = scale(32, self.dpi.get()).clamp(32, 64);
            let old = self.toolbar_images.get();
            let il = ImageList_Create(size, size, ILC_COLOR32 | ILC_MASK, 8, 0);
            let stock = [SIID_FOLDEROPEN, SIID_FOLDER, SIID_ZIPFILE, SIID_STACK, SIID_DELETE, SIID_SHIELD, SIID_FIND];
            for id in stock {
                let mut sii: SHSTOCKICONINFO = std::mem::zeroed();
                sii.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
                if SHGetStockIconInfo(id, SHGSI_ICON | SHGSI_LARGEICON, &mut sii) == 0 && !sii.hIcon.is_null() {
                    ImageList_ReplaceIcon(il, -1, sii.hIcon);
                    DestroyIcon(sii.hIcon);
                } else {
                    let fallback = LoadIconW(null_mut(), IDI_APPLICATION);
                    ImageList_ReplaceIcon(il, -1, fallback);
                }
            }
            SendMessageW(tb, TB_SETIMAGELIST, 0, il as isize);
            self.toolbar_images.set(il);
            if old != 0 {
                ImageList_Destroy(old);
            }
            // Remove existing buttons (DPI rebuild)
            while SendMessageW(tb, TB_DELETEBUTTON, 0, 0) != 0 {}
            let labels = [S::TbOpen, S::TbExtract, S::TbNew, S::TbAdd, S::TbDelete, S::TbTest, S::TbView];
            let cmds = [ID_OPEN, ID_EXTRACT, ID_NEW, ID_ADD, ID_DELETE, ID_TEST, ID_VIEW];
            let mut buttons: Vec<TBBUTTON> = Vec::new();
            for (i, (label, cmd)) in labels.iter().zip(cmds).enumerate() {
                let mut s: Vec<u16> = tr(*label).encode_utf16().collect();
                s.extend([0, 0]);
                let idx = SendMessageW(tb, TB_ADDSTRINGW, 0, s.as_ptr() as isize);
                let mut b: TBBUTTON = std::mem::zeroed();
                b.iBitmap = i as i32;
                b.idCommand = cmd;
                b.fsState = TBSTATE_ENABLED as u8;
                b.fsStyle = (BTNS_BUTTON | BTNS_AUTOSIZE) as u8;
                b.iString = idx;
                buttons.push(b);
                if cmd == ID_NEW || cmd == ID_DELETE {
                    let mut sep: TBBUTTON = std::mem::zeroed();
                    sep.fsStyle = BTNS_SEP as u8;
                    sep.iBitmap = scale(8, self.dpi.get());
                    buttons.push(sep);
                }
            }
            SendMessageW(tb, TB_ADDBUTTONSW, buttons.len(), buttons.as_ptr() as isize);
            SendMessageW(tb, TB_AUTOSIZE, 0, 0);
        }
    }

    fn add_columns(&self) {
        let dpi = self.dpi.get();
        let cols = [
            (S::ColName, 280, LVCFMT_LEFT),
            (S::ColPacked, 100, LVCFMT_RIGHT),
            (S::ColSize, 100, LVCFMT_RIGHT),
            (S::ColType, 140, LVCFMT_LEFT),
            (S::ColModified, 130, LVCFMT_LEFT),
            (S::ColCrc, 120, LVCFMT_LEFT),
        ];
        for (i, (s, w, fmt)) in cols.iter().enumerate() {
            let mut text = wide(tr(*s));
            unsafe {
                let mut c: LVCOLUMNW = std::mem::zeroed();
                c.mask = LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM;
                c.fmt = *fmt;
                c.cx = scale(*w, dpi);
                c.pszText = text.as_mut_ptr();
                c.iSubItem = i as i32;
                SendMessageW(self.list.get(), LVM_INSERTCOLUMNW, i, &c as *const _ as isize);
            }
        }
        self.update_sort_arrow();
    }

    fn layout(&self) {
        unsafe {
            let hwnd = self.hwnd.get();
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let tb = self.toolbar.get();
            SendMessageW(tb, TB_AUTOSIZE, 0, 0);
            let mut trc: RECT = std::mem::zeroed();
            GetWindowRect(tb, &mut trc);
            let top = trc.bottom - trc.top;
            let st = self.status.get();
            SendMessageW(st, WM_SIZE, 0, 0);
            let mut src: RECT = std::mem::zeroed();
            GetWindowRect(st, &mut src);
            let sh = src.bottom - src.top;
            let dpi = self.dpi.get();
            let parts = [scale(300, dpi), scale(560, dpi), scale(700, dpi), -1];
            SendMessageW(st, SB_SETPARTS, parts.len(), parts.as_ptr() as isize);
            let w = rc.right;
            let h = (rc.bottom - top - sh).max(0);
            if self.show_tree.get() {
                let sx = scale(self.split.get(), dpi).clamp(scale(80, dpi), (w - scale(160, dpi)).max(scale(80, dpi)));
                let gap = scale(SPLIT_GAP, dpi);
                ShowWindow(self.tree.get(), SW_SHOW);
                MoveWindow(self.tree.get(), 0, top, sx, h, 1);
                MoveWindow(self.list.get(), sx + gap, top, (w - sx - gap).max(0), h, 1);
            } else {
                ShowWindow(self.tree.get(), SW_HIDE);
                MoveWindow(self.list.get(), 0, top, w, h, 1);
            }
        }
    }

    // ------------------------------------------------------------ UI state

    fn update_ui(&self) {
        let (has, writable) = match &*self.model.borrow() {
            Some(m) => (true, m.writable()),
            None => (false, false),
        };
        let tb = self.toolbar.get();
        let menu = self.menu.get();
        let set = |cmd: i32, on: bool| unsafe {
            SendMessageW(tb, TB_ENABLEBUTTON, cmd as usize, on as isize);
            EnableMenuItem(menu, cmd as u32, MF_BYCOMMAND | if on { MF_ENABLED } else { MF_GRAYED });
        };
        set(ID_ADD, writable);
        set(ID_ADD_FOLDER, writable);
        set(ID_DELETE, writable);
        set(ID_TEST, has);
        set(ID_VIEW, has);
        set(ID_CLOSE, has);
        set(ID_EXTRACT_HERE, has);
        set(ID_EXTRACT_SMART, has);
        set(ID_PROPERTIES, has);
        set(ID_OPEN_FOLDER, has);
        set(ID_SELECT_ALL, has);
        set(ID_REFRESH, has);
        set(ID_UP, has);
        unsafe {
            CheckMenuItem(menu, ID_TOGGLE_TREE as u32, MF_BYCOMMAND | if self.show_tree.get() { MF_CHECKED } else { MF_UNCHECKED });
        }
        let title = match &*self.model.borrow() {
            Some(m) => format!(
                "{} - {}",
                m.info.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                tr(S::AppName)
            ),
            None => tr(S::AppName).to_string(),
        };
        set_text(self.hwnd.get(), &title);
    }

    fn set_status(&self, part: usize, text: &str) {
        let w = wide(text);
        unsafe { SendMessageW(self.status.get(), SB_SETTEXTW, part, w.as_ptr() as isize) };
    }

    fn update_status(&self) {
        let model = self.model.borrow();
        let Some(m) = &*model else {
            self.set_status(0, tr(S::StReady));
            for i in 1..4 {
                self.set_status(i, "");
            }
            return;
        };
        let sel = self.selected_rows().iter().filter(|r| **r != Row::Up).count();
        let summary = trf(S::StSummary, &[("files", &m.info.file_count().to_string()), ("dirs", &(m.folders.len() - 1).to_string())]);
        let first = if sel > 0 { format!("{summary} · {}", trf(S::StSelected, &[("n", &sel.to_string())])) } else { summary };
        self.set_status(0, &first);
        let size = m.info.total_size();
        let packed = m.info.total_packed();
        self.set_status(1, &trf(S::StSizes, &[("size", &format_size(size)), ("packed", &format_size(packed))]));
        let ratio = if size > 0 { format!("{:.1}%", packed as f64 * 100.0 / size as f64) } else { "-".into() };
        self.set_status(2, &trf(S::StRatio, &[("ratio", &ratio)]));
        let mut flags = vec![m.info.format.name().to_string()];
        if m.info.encrypted {
            flags.push(tr(S::StEncrypted).into());
        }
        if m.info.solid {
            flags.push(tr(S::StSolid).into());
        }
        if m.info.volumes > 1 {
            flags.push(trf(S::StVolumes, &[("n", &m.info.volumes.to_string())]));
        }
        self.set_status(3, &flags.join(" · "));
    }

    // ------------------------------------------------------------ archive

    fn open_archive(&self, path: &Path) {
        let hwnd = self.hwnd.get();
        let path = path.to_path_buf();
        let mut pw = None;
        let p2 = path.clone();
        let result = with_password(hwnd, &path, &mut pw, |pw| {
            let p3 = p2.clone();
            run_task(hwnd, tr(S::GOpening), move |prog, _| list(&p3, &OpenOptions { password: pw }, &prog))
        });
        match result {
            Ok(info) => {
                *self.password.borrow_mut() = pw;
                self.set_model(Model::build(info), "");
            }
            Err(e) => report_error(hwnd, &path.display().to_string(), &e),
        }
    }

    /// Re-list the current archive (after an edit), keeping the current folder.
    fn reload(&self) {
        let (path, folder) = match &*self.model.borrow() {
            Some(m) => (m.info.path.clone(), m.folders[self.cur.get()].path.clone()),
            None => return,
        };
        let hwnd = self.hwnd.get();
        let pw = self.password.borrow().clone();
        let p2 = path.clone();
        match run_task(hwnd, tr(S::GOpening), move |prog, _| list(&p2, &OpenOptions { password: pw }, &prog)) {
            Ok(info) => self.set_model(Model::build(info), &folder),
            Err(e) => {
                report_error(hwnd, "", &e);
                self.close_archive();
            }
        }
    }

    fn set_model(&self, model: Model, folder_path: &str) {
        *self.model.borrow_mut() = Some(model);
        self.populate_tree();
        let target = self.model.borrow().as_ref().and_then(|m| m.folder_by_path(folder_path)).unwrap_or(0);
        self.navigate(target, true);
        self.update_ui();
    }

    fn close_archive(&self) {
        *self.model.borrow_mut() = None;
        *self.password.borrow_mut() = None;
        self.rows.borrow_mut().clear();
        unsafe {
            SendMessageW(self.tree.get(), TVM_DELETEITEM, 0, TVI_ROOT);
            SendMessageW(self.list.get(), LVM_SETITEMCOUNT, 0, 0);
        }
        self.update_ui();
        self.update_status();
    }

    fn populate_tree(&self) {
        let tree = self.tree.get();
        unsafe { SendMessageW(tree, TVM_DELETEITEM, 0, TVI_ROOT) };
        let model = self.model.borrow();
        let Some(m) = &*model else { return };
        let closed = icon_for("", true, false).icon;
        let open = icon_for("", true, true).icon;
        let root_icon = icon_for_path(&m.info.path);
        let root_name = m.info.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        fn insert(tree: HWND, parent: isize, text: &str, img: i32, sel: i32, id: usize) -> isize {
            let mut t = wide(text);
            unsafe {
                let mut tvi: TVINSERTSTRUCTW = std::mem::zeroed();
                tvi.hParent = parent as HTREEITEM;
                tvi.hInsertAfter = TVI_LAST;
                tvi.Anonymous.item = TVITEMW {
                    mask: TVIF_TEXT | TVIF_IMAGE | TVIF_SELECTEDIMAGE | TVIF_PARAM,
                    hItem: 0,
                    state: 0,
                    stateMask: 0,
                    pszText: t.as_mut_ptr(),
                    cchTextMax: 0,
                    iImage: img,
                    iSelectedImage: sel,
                    cChildren: 0,
                    lParam: id as isize,
                };
                SendMessageW(tree, TVM_INSERTITEMW, 0, &tvi as *const _ as isize)
            }
        }
        let root = insert(tree, TVI_ROOT, &root_name, root_icon, root_icon, 0);
        m.folders[0].htree.set(root);
        // breadth-first so parents exist before children
        let mut queue = vec![0usize];
        while let Some(f) = queue.pop() {
            let parent_h = m.folders[f].htree.get();
            for &s in &m.folders[f].subs {
                let h = insert(tree, parent_h, &m.folders[s].name, closed, open, s);
                m.folders[s].htree.set(h);
                queue.push(s);
            }
        }
        unsafe { SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as usize, root) };
    }

    fn navigate(&self, folder: usize, force: bool) {
        if !force && folder == self.cur.get() {
            return;
        }
        self.cur.set(folder);
        self.rebuild_rows();
        let h = self.model.borrow().as_ref().map(|m| m.folders.get(folder).map(|f| f.htree.get()).unwrap_or(0)).unwrap_or(0);
        if h != 0 {
            unsafe { SendMessageW(self.tree.get(), TVM_SELECTITEM, TVGN_CARET as usize, h) };
        }
        self.update_status();
    }

    fn rebuild_rows(&self) {
        let rows = {
            let model = self.model.borrow();
            let Some(m) = &*model else { return };
            let f = &m.folders[self.cur.get().min(m.folders.len() - 1)];
            let mut dirs: Vec<Row> = f.subs.iter().map(|&s| Row::Folder(s)).collect();
            let mut files: Vec<Row> = f.files.iter().map(|&i| Row::File(i)).collect();
            let (col, asc) = self.sort.get();
            let key_cmp = |a: &Row, b: &Row| -> std::cmp::Ordering {
                use std::cmp::Ordering;
                let o = match (a, b) {
                    (Row::Folder(x), Row::Folder(y)) => {
                        let (fx, fy) = (&m.folders[*x], &m.folders[*y]);
                        match col {
                            1 => fx.packed.cmp(&fy.packed),
                            2 => fx.size.cmp(&fy.size),
                            4 => fx.mtime.cmp(&fy.mtime),
                            _ => fx.name.to_lowercase().cmp(&fy.name.to_lowercase()),
                        }
                    }
                    (Row::File(x), Row::File(y)) => {
                        let (ex, ey) = (&m.info.entries[*x], &m.info.entries[*y]);
                        match col {
                            1 => ex.packed_size.cmp(&ey.packed_size),
                            2 => ex.size.cmp(&ey.size),
                            3 => ext_of(ex.name()).cmp(&ext_of(ey.name())).then_with(|| ex.name().to_lowercase().cmp(&ey.name().to_lowercase())),
                            4 => ex.modified.cmp(&ey.modified),
                            5 => ex.crc32.cmp(&ey.crc32),
                            _ => ex.name().to_lowercase().cmp(&ey.name().to_lowercase()),
                        }
                    }
                    _ => Ordering::Equal,
                };
                if asc {
                    o
                } else {
                    o.reverse()
                }
            };
            dirs.sort_by(key_cmp);
            files.sort_by(key_cmp);
            let mut rows = Vec::with_capacity(dirs.len() + files.len() + 1);
            if self.cur.get() != 0 {
                rows.push(Row::Up);
            }
            rows.extend(dirs);
            rows.extend(files);
            rows
        };
        let n = rows.len();
        *self.rows.borrow_mut() = rows;
        unsafe {
            let list = self.list.get();
            SendMessageW(list, LVM_SETITEMCOUNT, n, 0);
            // clear selection
            let mut it: LVITEMW = std::mem::zeroed();
            it.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
            it.state = 0;
            SendMessageW(list, LVM_SETITEMSTATE, usize::MAX, &it as *const _ as isize);
            InvalidateRect(list, null(), 1);
        }
    }

    fn update_sort_arrow(&self) {
        unsafe {
            let header = SendMessageW(self.list.get(), LVM_GETHEADER, 0, 0) as HWND;
            let (col, asc) = self.sort.get();
            for i in 0..6 {
                let mut hd: HDITEMW = std::mem::zeroed();
                hd.mask = HDI_FORMAT;
                SendMessageW(header, HDM_GETITEMW, i, &mut hd as *mut _ as isize);
                hd.fmt &= !(HDF_SORTUP | HDF_SORTDOWN);
                if i as i32 == col {
                    hd.fmt |= if asc { HDF_SORTUP } else { HDF_SORTDOWN };
                }
                SendMessageW(header, HDM_SETITEMW, i, &hd as *const _ as isize);
            }
        }
    }

    fn selected_rows(&self) -> Vec<Row> {
        let rows = self.rows.borrow();
        let mut out = vec![];
        let mut i: isize = -1;
        loop {
            let next = unsafe { SendMessageW(self.list.get(), LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as isize) };
            if next < 0 {
                break;
            }
            if let Some(r) = rows.get(next as usize) {
                out.push(*r);
            }
            i = next;
        }
        out
    }

    /// Archive paths of the selected rows (folders select their subtree).
    fn selected_paths(&self) -> Vec<String> {
        let model = self.model.borrow();
        let Some(m) = &*model else { return vec![] };
        self.selected_rows()
            .into_iter()
            .filter_map(|r| match r {
                Row::Folder(f) => Some(m.folders[f].path.clone()),
                Row::File(i) => Some(m.info.entries[i].path.clone()),
                Row::Up => None,
            })
            .collect()
    }

    fn current_folder_path(&self) -> String {
        self.model.borrow().as_ref().map(|m| m.folders[self.cur.get()].path.clone()).unwrap_or_default()
    }

    // ------------------------------------------------------------ list view callbacks

    fn disp_info(&self, di: &mut NMLVDISPINFOW) {
        let rows = self.rows.borrow();
        let Some(row) = rows.get(di.item.iItem as usize).copied() else { return };
        let model = self.model.borrow();
        let Some(m) = &*model else { return };
        let item = &mut di.item;
        if item.mask & LVIF_IMAGE != 0 {
            item.iImage = match row {
                Row::Up | Row::Folder(_) => icon_for("", true, false).icon,
                Row::File(i) => icon_for(m.info.entries[i].name(), false, false).icon,
            };
        }
        if item.mask & LVIF_TEXT != 0 && !item.pszText.is_null() && item.cchTextMax > 0 {
            let text: String = match (row, item.iSubItem) {
                (Row::Up, 0) => tr(S::UpRow).into(),
                (Row::Up, _) => String::new(),
                (Row::Folder(f), c) => {
                    let fo = &m.folders[f];
                    match c {
                        0 => fo.name.clone(),
                        1 => {
                            if fo.packed > 0 {
                                format_size(fo.packed)
                            } else {
                                String::new()
                            }
                        }
                        2 => format_size(fo.size),
                        3 => {
                            let t = icon_for("", true, false).type_name;
                            if t.is_empty() {
                                tr(S::FolderType).into()
                            } else {
                                t
                            }
                        }
                        4 => format_time(fo.mtime),
                        _ => String::new(),
                    }
                }
                (Row::File(i), c) => {
                    let e = &m.info.entries[i];
                    match c {
                        0 => {
                            if e.encrypted {
                                format!("{}*", e.name())
                            } else {
                                e.name().to_string()
                            }
                        }
                        1 => e.packed_size.map(format_size).unwrap_or_default(),
                        2 => {
                            if e.size == 0 && m.info.format.is_single_stream() && m.info.format != ArchiveFormat::Gz {
                                "?".into()
                            } else {
                                format_size(e.size)
                            }
                        }
                        3 => icon_for(e.name(), false, false).type_name,
                        4 => format_time(e.modified),
                        5 => {
                            let crc = e.crc32.map(|c| format!("{c:08X}")).unwrap_or_default();
                            let attr = e.attributes.map(attr_string).unwrap_or_default();
                            format!("{crc}  {attr}").trim().to_string()
                        }
                        _ => String::new(),
                    }
                }
            };
            let w: Vec<u16> = text.encode_utf16().collect();
            let n = w.len().min(item.cchTextMax as usize - 1);
            unsafe {
                std::ptr::copy_nonoverlapping(w.as_ptr(), item.pszText, n);
                *item.pszText.add(n) = 0;
            }
        }
    }

    fn activate_row(&self, idx: usize) {
        let row = self.rows.borrow().get(idx).copied();
        match row {
            Some(Row::Up) => self.go_up(),
            Some(Row::Folder(f)) => self.navigate(f, false),
            Some(Row::File(i)) => self.open_entry(i),
            None => {}
        }
    }

    fn go_up(&self) {
        let parent = self.model.borrow().as_ref().and_then(|m| m.folders[self.cur.get()].parent);
        if let Some(p) = parent {
            self.navigate(p, false);
        }
    }

    /// Extract one entry to a private temp folder and open it with its app.
    fn open_entry(&self, index: usize) {
        let hwnd = self.hwnd.get();
        let (archive, entry_path, parent) = {
            let model = self.model.borrow();
            let Some(m) = &*model else { return };
            let e = &m.info.entries[index];
            (m.info.path.clone(), e.path.clone(), e.parent().to_string())
        };
        let n = self.temp_counter.get() + 1;
        self.temp_counter.set(n);
        let dest = self.temp_root.join(n.to_string());
        let mut pw = self.password.borrow().clone();
        let a2 = archive.clone();
        let result = with_password(hwnd, &archive, &mut pw, |pw| {
            let (a3, d, ep, pp) = (a2.clone(), dest.clone(), entry_path.clone(), parent.clone());
            run_task(hwnd, tr(S::GExtracting), move |prog, _| {
                let mut o = ExtractOptions::new().password(pw).overwrite(OverwriteMode::Overwrite);
                o.selection = Some(vec![ep]);
                o.strip_prefix = Some(pp);
                extract(&a3, &d, &o, &prog)
            })
        });
        match result {
            Ok(stats) => {
                *self.password.borrow_mut() = pw;
                if let Some(f) = stats.outputs.first() {
                    shell_open(hwnd, f);
                }
            }
            Err(e) => report_error(hwnd, "", &e),
        }
    }

    // ------------------------------------------------------------ commands

    fn cmd_open(&self) {
        if let Some(p) = open_files(self.hwnd.get(), tr(S::MiOpen).split('\t').next().unwrap_or(""), true, false).into_iter().next() {
            self.open_archive(&p);
        }
    }

    fn ensure_archive(&self) -> bool {
        if self.model.borrow().is_none() {
            self.cmd_open();
        }
        self.model.borrow().is_some()
    }

    /// mode 0 = dialog, 1 = here, 2 = smart sub-folder
    fn cmd_extract(&self, mode: u8) {
        if !self.ensure_archive() {
            return;
        }
        let hwnd = self.hwnd.get();
        let (archive, parent, stem, count, smart) = {
            let model = self.model.borrow();
            let m = model.as_ref().unwrap();
            let parent = m.info.path.parent().map(Path::to_path_buf).unwrap_or_default();
            (m.info.path.clone(), parent.clone(), archive_stem(&m.info.path), m.info.file_count(), smart_extract_dir(&m.info, &parent))
        };
        let selection = self.selected_paths();
        let selected_only = mode == 0 && !selection.is_empty();
        let (dest, overwrite, open_after, mut pw) = match mode {
            0 => {
                let scope = if selected_only {
                    trf(S::XScopeSel, &[("n", &selection.len().to_string())])
                } else {
                    trf(S::XScopeAll, &[("n", &count.to_string())])
                };
                let Some(c) = extract_dialog(hwnd, &parent, &stem, &scope, selected_only, self.password.borrow().clone()) else { return };
                let pw = c.password.or_else(|| self.password.borrow().clone());
                (c.dest, c.overwrite, c.open_after, pw)
            }
            1 => (parent.clone(), OverwriteMode::Ask, false, self.password.borrow().clone()),
            _ => (smart, OverwriteMode::Ask, false, self.password.borrow().clone()),
        };
        let strip = if selected_only { Some(self.current_folder_path()) } else { None };
        let sel = if selected_only { Some(selection) } else { None };
        let a2 = archive.clone();
        let d2 = dest.clone();
        let result = with_password(hwnd, &archive, &mut pw, |pw| {
            let (a3, d3, sel, strip) = (a2.clone(), d2.clone(), sel.clone(), strip.clone());
            run_task(hwnd, tr(S::GExtracting), move |prog, dlg| {
                let mut o = ExtractOptions::new().password(pw).overwrite(overwrite);
                o.on_conflict = Some(conflict_prompt(dlg));
                o.selection = sel;
                o.strip_prefix = strip;
                extract(&a3, &d3, &o, &prog)
            })
        });
        match result {
            Ok(stats) => {
                *self.password.borrow_mut() = pw;
                self.set_status(0, &trf(S::MExtractDone, &[("n", &stats.files.to_string()), ("dest", &dest.display().to_string())]));
                if open_after {
                    shell_open(hwnd, &dest);
                }
            }
            Err(e) => report_error(hwnd, "", &e),
        }
    }

    fn cmd_new(&self, sources: Vec<PathBuf>) {
        let hwnd = self.hwnd.get();
        let sources = if sources.is_empty() { open_files(hwnd, tr(S::MPickFiles), false, true) } else { sources };
        if sources.is_empty() {
            return;
        }
        if let Some(target) = create_archive_interactive(hwnd, &sources, ArchiveFormat::Zip) {
            self.open_archive(&target);
        }
    }

    fn check_writable(&self) -> bool {
        let hwnd = self.hwnd.get();
        let model = self.model.borrow();
        let Some(m) = &*model else {
            error_box(hwnd, tr(S::MNoArchive));
            return false;
        };
        if m.info.volumes > 1 || volume_set(&m.info.path).is_some() {
            error_box(hwnd, tr(S::MSplitReadOnly));
            return false;
        }
        if !m.info.format.is_writable() {
            error_box(hwnd, &trf(S::MReadOnly, &[("fmt", m.info.format.name())]));
            return false;
        }
        true
    }

    fn modify_archive(&self, ops: ModifyOps) {
        let hwnd = self.hwnd.get();
        let (archive, fmt) = match &*self.model.borrow() {
            Some(m) => (m.info.path.clone(), m.info.format),
            None => return,
        };
        let mut pw = self.password.borrow().clone();
        let a2 = archive.clone();
        let result = with_password(hwnd, &archive, &mut pw, |pw| {
            let (a3, ops) = (a2.clone(), ops.clone());
            run_task(hwnd, tr(S::GUpdating), move |prog, _| {
                let opts = CreateOptions { format: fmt, level: 5, password: pw, ..Default::default() };
                modify(&a3, &ops, &opts, &prog)
            })
        });
        match result {
            Ok(()) => {
                *self.password.borrow_mut() = pw;
                self.reload();
            }
            Err(e) => {
                report_error(hwnd, "", &e);
                self.reload();
            }
        }
    }

    fn cmd_add(&self, files: Vec<PathBuf>) {
        if !self.check_writable() {
            return;
        }
        let files = if files.is_empty() { open_files(self.hwnd.get(), tr(S::MPickAddFiles), false, true) } else { files };
        if files.is_empty() {
            return;
        }
        let into = self.current_folder_path();
        self.modify_archive(ModifyOps { add: files, add_into: into, delete: vec![] });
    }

    fn cmd_add_folder(&self) {
        if !self.check_writable() {
            return;
        }
        let start = self.model.borrow().as_ref().and_then(|m| m.info.path.parent().map(Path::to_path_buf)).unwrap_or_default();
        if let Some(dir) = browse_folder(self.hwnd.get(), tr(S::MPickAddFolder), &start) {
            self.cmd_add(vec![dir]);
        }
    }

    fn cmd_delete(&self) {
        let hwnd = self.hwnd.get();
        let sel = self.selected_paths();
        if sel.is_empty() {
            if self.model.borrow().is_some() {
                error_box(hwnd, tr(S::MNoSelection));
            }
            return;
        }
        if !self.check_writable() {
            return;
        }
        if !confirm(hwnd, &trf(S::MDeleteConfirm, &[("n", &sel.len().to_string())])) {
            return;
        }
        self.modify_archive(ModifyOps { delete: sel, ..Default::default() });
    }

    fn cmd_test(&self) {
        if !self.ensure_archive() {
            return;
        }
        let hwnd = self.hwnd.get();
        let archive = self.model.borrow().as_ref().map(|m| m.info.path.clone()).unwrap_or_default();
        let mut pw = self.password.borrow().clone();
        let a2 = archive.clone();
        let result = with_password(hwnd, &archive, &mut pw, |pw| {
            let a3 = a2.clone();
            run_task(hwnd, tr(S::GTesting), move |prog, _| okzip_core::test(&a3, pw.as_deref(), &prog))
        });
        match result {
            Ok(r) if r.ok() => {
                *self.password.borrow_mut() = pw;
                info_box(hwnd, &trf(S::MTestOk, &[("n", &r.files_tested.to_string()), ("size", &format_size(r.bytes_tested))]));
            }
            Ok(r) => {
                let list: Vec<String> = r.errors.iter().take(20).map(|(n, m)| format!("• {n}: {m}")).collect();
                message_box(hwnd, &trf(S::MTestFail, &[("n", &r.errors.len().to_string()), ("list", &list.join("\n"))]), MB_OK | MB_ICONWARNING);
            }
            Err(e) => report_error(hwnd, "", &e),
        }
    }

    fn cmd_view(&self) {
        let rows = self.selected_rows();
        if let Some(first) = rows.first() {
            let idx = self.rows.borrow().iter().position(|r| r == first);
            if let Some(i) = idx {
                self.activate_row(i);
            }
        } else if self.model.borrow().is_some() {
            error_box(self.hwnd.get(), tr(S::MNoSelection));
        }
    }

    fn cmd_properties(&self) {
        let text = {
            let model = self.model.borrow();
            let Some(m) = &*model else { return };
            let size = m.info.total_size();
            let packed = m.info.total_packed();
            let mut flags = String::new();
            if m.info.encrypted {
                flags.push_str(&format!(", {}", tr(S::StEncrypted)));
            }
            if m.info.solid {
                flags.push_str(&format!(", {}", tr(S::StSolid)));
            }
            trf(
                S::MProps,
                &[
                    ("path", &m.info.path.display().to_string()),
                    ("fmt", m.info.format.name()),
                    ("flags", &flags),
                    ("vols", &m.info.volumes.to_string()),
                    ("asize", &format_size(m.info.archive_size)),
                    ("files", &m.info.file_count().to_string()),
                    ("dirs", &(m.folders.len() - 1).to_string()),
                    ("size", &format_size(size)),
                    ("ratio", &if size > 0 { format!("{:.1}%", packed as f64 * 100.0 / size as f64) } else { "-".into() }),
                ],
            )
        };
        info_box(self.hwnd.get(), &text);
    }

    fn cmd_select_all(&self) {
        unsafe {
            let mut it: LVITEMW = std::mem::zeroed();
            it.stateMask = LVIS_SELECTED;
            it.state = LVIS_SELECTED;
            SendMessageW(self.list.get(), LVM_SETITEMSTATE, usize::MAX, &it as *const _ as isize);
            SetFocus(self.list.get());
        }
    }

    fn cmd_about(&self) {
        let read: Vec<&str> = ArchiveFormat::ALL.iter().filter(|f| **f != ArchiveFormat::Rar || RAR_SUPPORTED).map(|f| f.name()).collect();
        let write: Vec<&str> = ArchiveFormat::WRITABLE.iter().map(|f| f.name()).collect();
        info_box(
            self.hwnd.get(),
            &trf(S::MAbout, &[("ver", env!("CARGO_PKG_VERSION")), ("read", &read.join(", ")), ("write", &write.join(", "))]),
        );
    }

    fn on_command(&self, id: i32) {
        match id {
            ID_OPEN => self.cmd_open(),
            ID_EXTRACT => self.cmd_extract(0),
            ID_EXTRACT_HERE => self.cmd_extract(1),
            ID_EXTRACT_SMART => self.cmd_extract(2),
            ID_NEW => self.cmd_new(vec![]),
            ID_ADD => self.cmd_add(vec![]),
            ID_ADD_FOLDER => self.cmd_add_folder(),
            ID_DELETE => self.cmd_delete(),
            ID_TEST => self.cmd_test(),
            ID_VIEW => self.cmd_view(),
            ID_CLOSE => self.close_archive(),
            ID_EXIT => unsafe {
                DestroyWindow(self.hwnd.get());
            },
            ID_SELECT_ALL => self.cmd_select_all(),
            ID_REFRESH => self.reload(),
            ID_UP => self.go_up(),
            ID_TOGGLE_TREE => {
                self.show_tree.set(!self.show_tree.get());
                self.layout();
                self.update_ui();
            }
            ID_SETTINGS => settings_dialog(self.hwnd.get()),
            ID_ABOUT => self.cmd_about(),
            ID_PROPERTIES => self.cmd_properties(),
            ID_OPEN_FOLDER => {
                if let Some(p) = self.model.borrow().as_ref().map(|m| m.info.path.clone()) {
                    reveal_in_explorer(&p);
                }
            }
            _ => {}
        }
    }

    fn on_drop(&self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        let single_archive = files.len() == 1 && files[0].is_file() && detect_format(&files[0]).is_ok();
        if single_archive {
            self.open_archive(&files[0]);
            return;
        }
        let writable = self.model.borrow().as_ref().map(|m| m.writable()).unwrap_or(false);
        if writable {
            let q = trf(S::MAddToArchiveQ, &[("n", &files.len().to_string())]);
            match message_box(self.hwnd.get(), &q, MB_YESNOCANCEL | MB_ICONQUESTION) {
                IDYES => self.cmd_add(files),
                IDNO => self.cmd_new(files),
                _ => {}
            }
        } else {
            self.cmd_new(files);
        }
    }

    fn on_notify(&self, hdr: *const NMHDR) -> LRESULT {
        unsafe {
            let h = &*hdr;
            if h.hwndFrom == self.list.get() {
                match h.code {
                    LVN_GETDISPINFOW => self.disp_info(&mut *(hdr as *mut NMLVDISPINFOW)),
                    NM_DBLCLK => {
                        let ia = &*(hdr as *const NMITEMACTIVATE);
                        if ia.iItem >= 0 {
                            self.activate_row(ia.iItem as usize);
                        }
                    }
                    LVN_COLUMNCLICK => {
                        let nm = &*(hdr as *const NMLISTVIEW);
                        let (col, asc) = self.sort.get();
                        self.sort.set((nm.iSubItem, if col == nm.iSubItem { !asc } else { true }));
                        self.update_sort_arrow();
                        self.rebuild_rows();
                    }
                    LVN_ITEMCHANGED => self.update_status(),
                    NM_RCLICK => self.context_menu(),
                    _ => {}
                }
            } else if h.hwndFrom == self.tree.get() && h.code == TVN_SELCHANGEDW {
                let nm = &*(hdr as *const NMTREEVIEWW);
                let id = nm.itemNew.lParam as usize;
                let valid = self.model.borrow().as_ref().is_some_and(|m| id < m.folders.len());
                if valid {
                    self.navigate(id, false);
                }
            }
        }
        0
    }

    fn context_menu(&self) {
        if self.model.borrow().is_none() {
            return;
        }
        unsafe {
            let m = CreatePopupMenu();
            let add = |id: i32, s: S| {
                let w = wide(tr(s));
                AppendMenuW(m, MF_STRING, id as usize, w.as_ptr());
            };
            add(ID_VIEW, S::MiView);
            add(ID_EXTRACT, S::MiExtract);
            AppendMenuW(m, MF_SEPARATOR, 0, null());
            add(ID_DELETE, S::MiDelete);
            add(ID_SELECT_ALL, S::MiSelectAll);
            let writable = self.model.borrow().as_ref().is_some_and(|m| m.writable());
            if !writable {
                EnableMenuItem(m, ID_DELETE as u32, MF_BYCOMMAND | MF_GRAYED);
            }
            let mut pt: POINT = std::mem::zeroed();
            GetCursorPos(&mut pt);
            let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, self.hwnd.get(), null());
            DestroyMenu(m);
            if cmd != 0 {
                self.on_command(cmd);
            }
        }
    }

    fn on_destroy(&self) {
        let _ = std::fs::remove_dir_all(&self.temp_root);
        unsafe { PostQuitMessage(0) };
    }
}

fn ext_of(name: &str) -> String {
    name.rfind('.').map(|i| name[i + 1..].to_lowercase()).unwrap_or_default()
}

fn build_menu() -> HMENU {
    unsafe {
        let bar = CreateMenu();
        let sub = |title: S, items: &[(i32, Option<S>)]| {
            let m = CreatePopupMenu();
            for (id, s) in items {
                match s {
                    Some(s) => {
                        let w = wide(tr(*s));
                        AppendMenuW(m, MF_STRING, *id as usize, w.as_ptr());
                    }
                    None => {
                        AppendMenuW(m, MF_SEPARATOR, 0, null());
                    }
                }
            }
            let t = wide(tr(title));
            AppendMenuW(bar, MF_POPUP, m as usize, t.as_ptr());
        };
        sub(
            S::MenuFile,
            &[
                (ID_OPEN, Some(S::MiOpen)),
                (ID_NEW, Some(S::MiNew)),
                (ID_CLOSE, Some(S::MiClose)),
                (0, None),
                (ID_OPEN_FOLDER, Some(S::MiOpenFolder)),
                (ID_PROPERTIES, Some(S::MiProperties)),
                (0, None),
                (ID_EXIT, Some(S::MiExit)),
            ],
        );
        sub(
            S::MenuEdit,
            &[
                (ID_ADD, Some(S::MiAdd)),
                (ID_ADD_FOLDER, Some(S::MiAddFolder)),
                (ID_DELETE, Some(S::MiDelete)),
                (0, None),
                (ID_SELECT_ALL, Some(S::MiSelectAll)),
            ],
        );
        sub(
            S::MenuView,
            &[(ID_VIEW, Some(S::MiView)), (ID_UP, Some(S::MiUp)), (ID_REFRESH, Some(S::MiRefresh)), (0, None), (ID_TOGGLE_TREE, Some(S::MiToggleTree))],
        );
        sub(
            S::MenuTools,
            &[
                (ID_EXTRACT, Some(S::MiExtract)),
                (ID_EXTRACT_HERE, Some(S::MiExtractHere)),
                (ID_EXTRACT_SMART, Some(S::MiExtractSmart)),
                (ID_TEST, Some(S::MiTest)),
                (0, None),
                (ID_SETTINGS, Some(S::MiSettings)),
            ],
        );
        sub(S::MenuHelp, &[(ID_ABOUT, Some(S::MiAbout))]);
        bar
    }
}

/// Show the "new archive" dialog and run the compression. Returns the path
/// to open afterwards (first volume for split archives).
pub fn create_archive_interactive(owner: HWND, sources: &[PathBuf], format: ArchiveFormat) -> Option<PathBuf> {
    let choice = create_dialog(owner, sources, format)?;
    let target = choice.target.clone();
    let opts = choice.options.clone();
    let srcs = sources.to_vec();
    match run_task(owner, tr(S::GCompressing), move |prog, _| create(&srcs, &target, &opts, &prog)) {
        Ok(stats) => stats.outputs.first().cloned(),
        Err(e) => {
            report_error(owner, "", &e);
            None
        }
    }
}

fn dropped_files(hdrop: HDROP) -> Vec<PathBuf> {
    unsafe {
        let n = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0);
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, null_mut(), 0);
            let mut buf = vec![0u16; len as usize + 1];
            DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32);
            out.push(path_from_wide(&buf));
        }
        DragFinish(hdrop);
        out
    }
}

fn x_from_lparam(lp: LPARAM) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(app) = app() else { return DefWindowProcW(hwnd, msg, wp, lp) };
        match msg {
            WM_CREATE => {
                app.on_create(hwnd);
                0
            }
            WM_SIZE => {
                app.layout();
                0
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(lp as *mut MINMAXINFO);
                let dpi = app.dpi.get();
                mmi.ptMinTrackSize.x = scale(520, dpi);
                mmi.ptMinTrackSize.y = scale(320, dpi);
                0
            }
            WM_DPICHANGED => {
                app.dpi.set(hiword(wp));
                let r = &*(lp as *const RECT);
                SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                app.apply_font();
                app.build_toolbar();
                app.layout();
                0
            }
            WM_COMMAND => {
                app.on_command(loword(wp) as i32);
                0
            }
            WM_NOTIFY => app.on_notify(lp as *const NMHDR),
            WM_DROPFILES => {
                let files = dropped_files(wp as HDROP);
                SetForegroundWindow(hwnd);
                app.on_drop(files);
                0
            }
            WM_SETCURSOR => {
                if wp as HWND == hwnd && loword(lp as usize) == HTCLIENT && app.show_tree.get() {
                    SetCursor(LoadCursorW(null_mut(), IDC_SIZEWE));
                    return 1;
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONDOWN => {
                if app.show_tree.get() {
                    app.dragging.set(true);
                    SetCapture(hwnd);
                }
                0
            }
            WM_MOUSEMOVE => {
                if app.dragging.get() {
                    let x = x_from_lparam(lp).max(0);
                    app.split.set((x as i64 * 96 / app.dpi.get() as i64) as i32);
                    app.layout();
                }
                0
            }
            WM_LBUTTONUP => {
                if app.dragging.get() {
                    app.dragging.set(false);
                    ReleaseCapture();
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                app.on_destroy();
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
