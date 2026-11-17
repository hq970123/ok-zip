//! The Bandizip-style main window: menu bar, big-button toolbar, folder tree,
//! virtual report list view with shell icons, status bar, splitter, drag & drop.

use super::band::{self, Btn, WM_BAND_DROPDOWN};
use super::theme;
use super::dialogs::{create_dialog, extract_dialog, settings_dialog};
use super::dragout;
use super::settings::{self, ExtractTarget};
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
    home_hot: Cell<i32>,
    home_tracking: Cell<bool>,
    was_home: Cell<bool>,
    recent_menu: Cell<HMENU>,
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
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: LoadImageW(hinst, make_int_resource(IDI_APP), IMAGE_ICON, small, small, LR_DEFAULTCOLOR) as HICON,
        };
        RegisterClassExW(&wc);
        let cfg = settings::get();

        let state: &'static App = Box::leak(Box::new(App {
            hwnd: Cell::new(null_mut()),
            toolbar: Cell::new(null_mut()),
            tree: Cell::new(null_mut()),
            list: Cell::new(null_mut()),
            status: Cell::new(null_mut()),
            menu: Cell::new(null_mut()),
            font: Cell::new(null_mut()),
            dpi: Cell::new(96),
            split: Cell::new(cfg.splitter.clamp(60, 2000)),
            dragging: Cell::new(false),
            show_tree: Cell::new(if cfg.remember_window { cfg.show_tree } else { true }),
            model: RefCell::new(None),
            password: RefCell::new(None),
            cur: Cell::new(0),
            rows: RefCell::new(vec![]),
            sort: Cell::new((0, true)),
            temp_root: std::env::temp_dir().join(format!("OKZip-{}", std::process::id())),
            temp_counter: Cell::new(0),
            home_hot: Cell::new(-1),
            home_tracking: Cell::new(false),
            was_home: Cell::new(true),
            recent_menu: Cell::new(null_mut()),
        }));
        APP.with(|a| a.set(Some(state)));

        let sys_dpi = windows_sys::Win32::UI::HiDpi::GetDpiForSystem().max(96);
        let title = wide(tr(S::AppName));
        // Restore the saved placement only when it is still on a monitor.
        let saved = cfg.window.filter(|r| {
            cfg.remember_window && r.w >= 300 && r.h >= 200 && {
                let rc = RECT { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h };
                !MonitorFromRect(&rc, MONITOR_DEFAULTTONULL).is_null()
            }
        });
        let (x, y, w, h) = match saved {
            Some(r) => (r.x, r.y, r.w, r.h),
            None => (CW_USEDEFAULT, CW_USEDEFAULT, scale(1000, sys_dpi), scale(640, sys_dpi)),
        };
        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        ShowWindow(hwnd, if saved.is_some_and(|r| r.maximized) { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL });
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
            let (menu, recent) = build_menu();
            self.menu.set(menu);
            self.recent_menu.set(recent);
            self.rebuild_recent_menu();
            SetMenu(hwnd, self.menu.get());

            // Command band (Bandizip-style blue strip with Lucide icons)
            let b = |cmd, icon, label, dropdown, right| Btn { cmd, icon, label, dropdown, right };
            let tb = band::create(
                hwnd,
                vec![
                    b(ID_OPEN, "open", S::TbOpen, true, false),
                    b(ID_EXTRACT_DEFAULT, "extract", S::TbExtract, true, false),
                    b(ID_NEW, "new", S::TbNew, false, false),
                    b(ID_ADD, "add", S::TbAdd, false, false),
                    b(ID_DELETE, "delete", S::TbDelete, false, false),
                    b(ID_TEST, "test", S::TbTest, false, false),
                    b(ID_VIEW, "view", S::TbView, false, false),
                    b(ID_SETTINGS, "settings", S::TbSettings, false, true),
                ],
            );
            self.toolbar.set(tb);

            // Tree
            let tree = CreateWindowExW(
                0,
                WC_TREEVIEWW,
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | TVS_HASBUTTONS | TVS_LINESATROOT | TVS_SHOWSELALWAYS | TVS_FULLROWSELECT | TVS_TRACKSELECT,
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
                0,
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
        for h in [self.tree.get(), self.list.get(), self.status.get()] {
            set_font(h, f);
        }
        band::set_font(self.toolbar.get(), f, self.dpi.get());
        unsafe {
            // Taller, Bandizip-like rows in the folder tree.
            let tree = self.tree.get();
            SendMessageW(tree, TVM_SETITEMHEIGHT, scale(24, self.dpi.get()) as usize, 0);
        }
        if !old.is_null() {
            unsafe { DeleteObject(old as HGDIOBJ) };
        }
    }

    fn build_toolbar(&self) {
        band::set_font(self.toolbar.get(), self.font.get(), self.dpi.get());
    }

    fn add_columns(&self) {
        let dpi = self.dpi.get();
        let saved = settings::get();
        let saved_widths = if saved.remember_window && saved.column_widths.len() == COLUMNS.len() { Some(saved.column_widths) } else { None };
        for (i, (s, w, fmt)) in COLUMNS.iter().enumerate() {
            let w = saved_widths.as_ref().map(|v| v[i].clamp(24, 1200)).unwrap_or(*w);
            let mut text = wide(tr(*s));
            unsafe {
                let mut c: LVCOLUMNW = std::mem::zeroed();
                c.mask = LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM;
                c.fmt = *fmt;
                c.cx = scale(w, dpi);
                c.pszText = text.as_mut_ptr();
                c.iSubItem = i as i32;
                SendMessageW(self.list.get(), LVM_INSERTCOLUMNW, i, &c as *const _ as isize);
            }
        }
        self.update_sort_arrow();
    }

    /// Current column widths in 96-DPI units.
    fn column_widths(&self) -> Vec<i32> {
        let dpi = self.dpi.get().max(96) as i64;
        (0..COLUMNS.len())
            .map(|i| {
                let px = unsafe { SendMessageW(self.list.get(), LVM_GETCOLUMNWIDTH, i, 0) } as i64;
                (px * 96 / dpi) as i32
            })
            .collect()
    }

    fn save_window_state(&self) {
        unsafe {
            let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
            wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            let have = GetWindowPlacement(self.hwnd.get(), &mut wp) != 0;
            let r = wp.rcNormalPosition;
            let widths = self.column_widths();
            let (split, tree) = (self.split.get(), self.show_tree.get());
            settings::update(|s| {
                if !s.remember_window {
                    return;
                }
                if have && r.right > r.left && r.bottom > r.top {
                    s.window = Some(settings::WindowRect {
                        x: r.left,
                        y: r.top,
                        w: r.right - r.left,
                        h: r.bottom - r.top,
                        maximized: wp.showCmd == SW_SHOWMAXIMIZED as u32,
                    });
                }
                s.column_widths = widths;
                s.splitter = split;
                s.show_tree = tree;
            });
        }
    }

    fn layout(&self) {
        unsafe {
            let hwnd = self.hwnd.get();
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let tb = self.toolbar.get();
            let home = self.is_home();
            let dpi = self.dpi.get();
            let top = if home { 0 } else { band::height(dpi) };
            MoveWindow(tb, 0, 0, rc.right, top, 1);
            ShowWindow(tb, if home { SW_HIDE } else { SW_SHOW });
            let st = self.status.get();
            SendMessageW(st, WM_SIZE, 0, 0);
            let mut src: RECT = std::mem::zeroed();
            GetWindowRect(st, &mut src);
            let sh = src.bottom - src.top;
            if home {
                let parts = [-1i32];
                SendMessageW(st, SB_SETPARTS, 1, parts.as_ptr() as isize);
            } else {
                let parts = [scale(300, dpi), scale(560, dpi), scale(700, dpi), -1];
                SendMessageW(st, SB_SETPARTS, parts.len(), parts.as_ptr() as isize);
            }
            let w = rc.right;
            let h = (rc.bottom - top - sh).max(0);
            if home {
                ShowWindow(self.tree.get(), SW_HIDE);
                ShowWindow(self.list.get(), SW_HIDE);
            } else if self.show_tree.get() {
                ShowWindow(self.list.get(), SW_SHOW);
                let sx = scale(self.split.get(), dpi).clamp(scale(80, dpi), (w - scale(160, dpi)).max(scale(80, dpi)));
                let gap = scale(SPLIT_GAP, dpi);
                ShowWindow(self.tree.get(), SW_SHOW);
                MoveWindow(self.tree.get(), 0, top, sx, h, 1);
                MoveWindow(self.list.get(), sx + gap, top, (w - sx - gap).max(0), h, 1);
            } else {
                ShowWindow(self.tree.get(), SW_HIDE);
                ShowWindow(self.list.get(), SW_SHOW);
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
            band::enable(tb, cmd, on);
            EnableMenuItem(menu, cmd as u32, MF_BYCOMMAND | if on { MF_ENABLED } else { MF_GRAYED });
        };
        let home = !has;
        if self.was_home.replace(home) != home {
            self.layout();
            unsafe { InvalidateRect(self.hwnd.get(), null(), 1) };
        }
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
        set(ID_EXTRACT, has);
        set(ID_EXTRACT_DEFAULT, has);
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
                settings::update(|s| s.add_recent(&path));
                self.rebuild_recent_menu();
            }
            Err(e) => {
                report_error(hwnd, &path.display().to_string(), &e);
                if !path.exists() {
                    settings::update(|s| s.recent.retain(|r| r != &path));
                    self.rebuild_recent_menu();
                }
            }
        }
    }

    /// Refill 文件 → 最近打开 from the settings.
    fn rebuild_recent_menu(&self) {
        let m = self.recent_menu.get();
        if m.is_null() {
            return;
        }
        unsafe {
            while GetMenuItemCount(m) > 0 {
                DeleteMenu(m, 0, MF_BYPOSITION);
            }
            let recent = settings::get().recent;
            if recent.is_empty() {
                let w = wide(tr(S::MiRecentEmpty));
                AppendMenuW(m, MF_STRING | MF_GRAYED, 0, w.as_ptr());
                return;
            }
            for (i, p) in recent.iter().enumerate().take(settings::MAX_RECENT) {
                let shown = shorten_path(&p.display().to_string(), 70).replace('&', "&&");
                let label = if i < 9 { format!("&{} {shown}", i + 1) } else { format!("1&0 {shown}") };
                let w = wide(&label);
                AppendMenuW(m, MF_STRING, (ID_RECENT_FIRST + i as i32) as usize, w.as_ptr());
            }
            AppendMenuW(m, MF_SEPARATOR, 0, null());
            let w = wide(tr(S::MiRecentClear));
            AppendMenuW(m, MF_STRING, ID_RECENT_CLEAR as usize, w.as_ptr());
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
                            6 => ex.attributes.cmp(&ey.attributes),
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
            for i in 0..COLUMNS.len() {
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
                        // Empty folders (no files anywhere below) show nothing in
                        // both size columns rather than "0 B".
                        1 if fo.packed > 0 => format_size(fo.packed),
                        2 if fo.size > 0 || fo.packed > 0 => format_size(fo.size),
                        1 | 2 => String::new(),
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
                        5 => e.crc32.map(|c| format!("{c:08X}")).unwrap_or_default(),
                        6 => e.attributes.map(attr_string).unwrap_or_default(),
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

    /// mode 0 = dialog, 1 = here, 2 = smart sub-folder, 3 = per settings (toolbar)
    fn cmd_extract(&self, mode: u8) {
        if !self.ensure_archive() {
            return;
        }
        let cfg = settings::get();
        let mode = if mode == 3 {
            match cfg.extract_target {
                ExtractTarget::CurrentFolder => 1,
                ExtractTarget::SubfolderSmart => 2,
                ExtractTarget::Ask => 0,
            }
        } else {
            mode
        };
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
            1 => (parent.clone(), cfg.overwrite.mode(), cfg.open_folder_after_extract, self.password.borrow().clone()),
            _ => (smart, cfg.overwrite.mode(), cfg.open_folder_after_extract, self.password.borrow().clone()),
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
        if let Some(target) = create_archive_interactive(hwnd, &sources, settings::get().format()) {
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
            ID_EXTRACT_DEFAULT => self.cmd_extract(3),
            ID_RECENT_CLEAR => {
                settings::update(|s| s.recent.clear());
                self.rebuild_recent_menu();
            }
            id if (ID_RECENT_FIRST..ID_RECENT_FIRST + settings::MAX_RECENT as i32).contains(&id) => {
                if let Some(p) = settings::get().recent.get((id - ID_RECENT_FIRST) as usize).cloned() {
                    self.open_archive(&p);
                }
            }
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
            ID_SETTINGS => {
                settings_dialog(self.hwnd.get(), 0);
                self.rebuild_recent_menu();
            }
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
        // Our own drag-out landing back on the window: ignore.
        if files.is_empty() || files.iter().all(|f| f.starts_with(&self.temp_root)) {
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
                    LVN_BEGINDRAG => self.drag_out(),
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

    /// Drag selected entries out to Explorer: extract them to a private temp
    /// folder first (with progress), then start an OLE drag of those files.
    fn drag_out(&self) {
        let selection = self.selected_paths();
        if selection.is_empty() {
            return;
        }
        let hwnd = self.hwnd.get();
        let Some(archive) = self.model.borrow().as_ref().map(|m| m.info.path.clone()) else { return };
        let n = self.temp_counter.get() + 1;
        self.temp_counter.set(n);
        let dest = self.temp_root.join(format!("drag-{n}"));
        let strip = self.current_folder_path();
        let mut pw = self.password.borrow().clone();
        let a2 = archive.clone();
        let result = with_password(hwnd, &archive, &mut pw, |pw| {
            let (a3, d, sel, strip) = (a2.clone(), dest.clone(), selection.clone(), strip.clone());
            run_task(hwnd, tr(S::GPreparingDrag), move |prog, _| {
                let mut o = ExtractOptions::new().password(pw).overwrite(OverwriteMode::Overwrite);
                o.selection = Some(sel);
                o.strip_prefix = Some(strip);
                extract(&a3, &d, &o, &prog)
            })
        });
        match result {
            Ok(_) => {
                *self.password.borrow_mut() = pw;
                // The top-level items of the temp folder are what was selected.
                let mut items: Vec<PathBuf> = std::fs::read_dir(&dest).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
                items.sort();
                // If the button was released while extracting, don't start a drag
                // that would drop wherever the cursor happens to be.
                let held = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000 != 0 };
                if held && !items.is_empty() {
                    dragout::drag_files(&items);
                }
            }
            Err(e) => report_error(hwnd, "", &e),
        }
    }


    // ------------------------------------------------------------ start page & painting

    fn is_home(&self) -> bool {
        self.model.borrow().is_none()
    }

    /// Client area above the status bar.
    fn content_rect(&self) -> RECT {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd.get(), &mut rc);
            let mut src: RECT = std::mem::zeroed();
            GetWindowRect(self.status.get(), &mut src);
            rc.bottom = (rc.bottom - (src.bottom - src.top)).max(0);
            rc
        }
    }

    /// The two start-page tiles: 打开压缩文件, 新建压缩文件.
    fn home_tiles(&self) -> [RECT; 2] {
        let dpi = self.dpi.get();
        let rc = self.content_rect();
        let (w, h, gap) = (scale(168, dpi), scale(140, dpi), scale(6, dpi));
        let left = (rc.right - (2 * w + gap)) / 2;
        let top = rc.top + ((rc.bottom - rc.top) - h) * 9 / 20;
        let r0 = RECT { left, top, right: left + w, bottom: top + h };
        let r1 = RECT { left: left + w + gap, top, right: left + 2 * w + gap, bottom: top + h };
        [r0, r1]
    }

    fn home_hit(&self, x: i32, y: i32) -> i32 {
        self.home_tiles().iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|i| i as i32).unwrap_or(-1)
    }

    fn splitter_hit(&self, x: i32, y: i32) -> bool {
        if self.is_home() || !self.show_tree.get() {
            return false;
        }
        let dpi = self.dpi.get();
        let mut rc: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetWindowRect(self.tree.get(), &mut rc) };
        let mut pt = POINT { x: rc.right, y: rc.top };
        unsafe { ScreenToClient(self.hwnd.get(), &mut pt) };
        x >= pt.x - 1 && x <= pt.x + scale(SPLIT_GAP, dpi) && y >= pt.y
    }

    fn paint(&self, hdc: HDC) {
        let dpi = self.dpi.get();
        let mut rc: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetClientRect(self.hwnd.get(), &mut rc) };
        let buf = theme::Buffered::new(hdc, rc.right, rc.bottom);
        let dc = buf.dc;
        if !self.is_home() {
            // Only the splitter gap is visible between the child windows.
            theme::fill(dc, &rc, theme::SPLITTER);
            return;
        }
        let content = self.content_rect();
        theme::gradient_v(dc, &content, theme::HOME_TOP, theme::HOME_BOTTOM);
        let font = self.font.get();
        let labels = [(S::HomeOpen, "home_open"), (S::HomeNew, "home_new")];
        let icon = scale(44, dpi);
        for (i, r) in self.home_tiles().iter().enumerate() {
            let hot = self.home_hot.get() == i as i32;
            theme::fill(dc, r, if hot { theme::TILE_HOT } else { theme::WHITE });
            theme::frame(dc, r, theme::TILE_BORDER);
            let (label, name) = labels[i];
            let iy = r.top + scale(30, dpi);
            theme::draw_icon(dc, name, r.left + (r.right - r.left - icon) / 2, iy, icon, theme::TILE_ICON, 255);
            let tr_ = RECT { left: r.left, top: iy + icon + scale(16, dpi), right: r.right, bottom: r.bottom };
            theme::text(dc, tr(label), &tr_, font, theme::TILE_TEXT, DT_CENTER | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);
        }
        let tiles = self.home_tiles();
        let hint = RECT { left: content.left, top: tiles[0].bottom + scale(22, dpi), right: content.right, bottom: content.bottom };
        theme::text(dc, tr(S::HomeHint), &hint, font, theme::HINT_TEXT, DT_CENTER | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);
    }

    fn set_home_hot(&self, i: i32) {
        if self.home_hot.replace(i) != i {
            let tiles = self.home_tiles();
            unsafe {
                for r in &tiles {
                    InvalidateRect(self.hwnd.get(), r, 0);
                }
            }
        }
    }

    /// Drop-down part of a band split button.
    fn band_dropdown(&self, cmd: i32, at: &RECT) {
        unsafe {
            let (menu, owned) = if cmd == ID_OPEN {
                (self.recent_menu.get(), false)
            } else {
                let m = CreatePopupMenu();
                for (id, s) in [(ID_EXTRACT, S::MiExtract), (ID_EXTRACT_HERE, S::MiExtractHere), (ID_EXTRACT_SMART, S::MiExtractSmart)] {
                    let w = wide(tr(s));
                    AppendMenuW(m, MF_STRING, id as usize, w.as_ptr());
                }
                (m, true)
            };
            TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_LEFTBUTTON, at.left, at.bottom, 0, self.hwnd.get(), null());
            if owned {
                DestroyMenu(menu);
            }
        }
    }

    fn on_destroy(&self) {
        self.save_window_state();
        let _ = std::fs::remove_dir_all(&self.temp_root);
        unsafe { PostQuitMessage(0) };
    }
}

fn ext_of(name: &str) -> String {
    name.rfind('.').map(|i| name[i + 1..].to_lowercase()).unwrap_or_default()
}

/// "C:\\very\\long\\path\\file.zip" → "C:\\very\\…\\file.zip" (at most `max` chars).
fn shorten_path(p: &str, max: usize) -> String {
    let chars: Vec<char> = p.chars().collect();
    if chars.len() <= max {
        return p.to_string();
    }
    let keep_tail = max * 2 / 3;
    let keep_head = max.saturating_sub(keep_tail + 1);
    let head: String = chars[..keep_head].iter().collect();
    let tail: String = chars[chars.len() - keep_tail..].iter().collect();
    format!("{head}…{tail}")
}

/// List columns: (title, default width at 96 DPI, alignment). The defaults sum
/// to ~730 px so every column fits the default window without a scrollbar.
const COLUMNS: [(S, i32, i32); 7] = [
    (S::ColName, 200, LVCFMT_LEFT),
    (S::ColPacked, 82, LVCFMT_RIGHT),
    (S::ColSize, 82, LVCFMT_RIGHT),
    (S::ColType, 105, LVCFMT_LEFT),
    (S::ColModified, 120, LVCFMT_LEFT),
    (S::ColCrc, 78, LVCFMT_LEFT),
    (S::ColAttr, 56, LVCFMT_LEFT),
];

/// The menu bar and the (initially empty) 最近打开 submenu.
fn build_menu() -> (HMENU, HMENU) {
    unsafe {
        let bar = CreateMenu();
        let recent = CreatePopupMenu();
        let sub = |title: S, items: &[(i32, Option<S>)]| {
            let m = CreatePopupMenu();
            for (id, s) in items {
                match s {
                    None if *id == -1 => {
                        let w = wide(tr(S::MiRecent));
                        AppendMenuW(m, MF_POPUP, recent as usize, w.as_ptr());
                    }
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
                (-1, None), // 最近打开 submenu
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
        (bar, recent)
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

fn y_from_lparam(lp: LPARAM) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
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
                InvalidateRect(hwnd, null(), 0);
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
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                app.paint(hdc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_BAND_DROPDOWN => {
                app.band_dropdown(wp as i32, &*(lp as *const RECT));
                0
            }
            WM_SETCURSOR => {
                if wp as HWND == hwnd && loword(lp as usize) == HTCLIENT {
                    let mut pt: POINT = std::mem::zeroed();
                    GetCursorPos(&mut pt);
                    ScreenToClient(hwnd, &mut pt);
                    if app.splitter_hit(pt.x, pt.y) {
                        SetCursor(LoadCursorW(null_mut(), IDC_SIZEWE));
                        return 1;
                    }
                    if app.is_home() && app.home_hit(pt.x, pt.y) >= 0 {
                        SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                        return 1;
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (x_from_lparam(lp), y_from_lparam(lp));
                if app.splitter_hit(x, y) {
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
                } else if app.is_home() {
                    if !app.home_tracking.get() {
                        let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        TrackMouseEvent(&mut t);
                        app.home_tracking.set(true);
                    }
                    app.set_home_hot(app.home_hit(x_from_lparam(lp), y_from_lparam(lp)));
                }
                0
            }
            WM_MOUSELEAVE => {
                app.home_tracking.set(false);
                app.set_home_hot(-1);
                0
            }
            WM_LBUTTONUP => {
                if app.dragging.get() {
                    app.dragging.set(false);
                    ReleaseCapture();
                } else if app.is_home() {
                    match app.home_hit(x_from_lparam(lp), y_from_lparam(lp)) {
                        0 => app.on_command(ID_OPEN),
                        1 => app.on_command(ID_NEW),
                        _ => {}
                    }
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
