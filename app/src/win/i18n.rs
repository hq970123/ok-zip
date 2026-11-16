//! UI strings: Simplified Chinese with an English fallback table.
//!
//! The language is Chinese when the Windows UI language is Chinese (or when
//! `OKZIP_LANG=zh`), English otherwise (`OKZIP_LANG=en` forces it).

use std::sync::OnceLock;

static ZH: OnceLock<bool> = OnceLock::new();

pub fn is_zh() -> bool {
    *ZH.get_or_init(|| {
        if let Ok(v) = std::env::var("OKZIP_LANG") {
            return v.to_ascii_lowercase().starts_with("zh");
        }
        // PRIMARYLANGID(LANG_CHINESE) == 0x04
        let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        (lang & 0x3FF) == 0x04
    })
}

macro_rules! strings {
    ($($key:ident => $zh:expr, $en:expr;)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, clippy::enum_variant_names)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum S { $($key),* }

        /// Look up a UI string in the active language.
        pub fn tr(s: S) -> &'static str {
            if is_zh() {
                match s { $(S::$key => $zh),* }
            } else {
                match s { $(S::$key => $en),* }
            }
        }
    };
}

/// `trf(S::X, &[("name", "foo")])` replaces `{name}` placeholders.
pub fn trf(s: S, args: &[(&str, &str)]) -> String {
    let mut out = tr(s).to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

strings! {
    AppName => "OK Zip", "OK Zip";

    MenuFile => "文件(&F)", "&File";
    MenuEdit => "编辑(&E)", "&Edit";
    MenuView => "查看(&V)", "&View";
    MenuTools => "工具(&T)", "&Tools";
    MenuHelp => "帮助(&H)", "&Help";

    MiOpen => "打开压缩文件(&O)...\tCtrl+O", "&Open archive...\tCtrl+O";
    MiNew => "新建压缩文件(&N)...\tCtrl+N", "&New archive...\tCtrl+N";
    MiClose => "关闭压缩文件(&C)", "&Close archive";
    MiOpenFolder => "打开所在文件夹(&L)", "Open containing fo&lder";
    MiProperties => "属性(&R)...", "P&roperties...";
    MiExit => "退出(&X)", "E&xit";
    MiAdd => "添加文件(&A)...", "&Add files...";
    MiAddFolder => "添加文件夹(&F)...", "Add &folder...";
    MiDelete => "删除(&D)\tDel", "&Delete\tDel";
    MiSelectAll => "全选(&A)\tCtrl+A", "Select &all\tCtrl+A";
    MiView => "查看/打开(&V)\tEnter", "&View / open\tEnter";
    MiRefresh => "刷新(&R)\tF5", "&Refresh\tF5";
    MiUp => "上一级(&U)\tBackspace", "&Up one level\tBackspace";
    MiToggleTree => "文件夹树(&T)", "Folder &tree";
    MiExtract => "解压到(&E)...\tCtrl+E", "&Extract to...\tCtrl+E";
    MiExtractHere => "解压到当前文件夹(&H)", "Extract &here";
    MiExtractSmart => "解压到同名文件夹(&S)", "Extract to &sub-folder";
    MiTest => "测试压缩文件(&T)\tCtrl+T", "&Test archive\tCtrl+T";
    MiSettings => "设置(&S)...", "&Settings...";
    MiAbout => "关于 OK Zip(&A)", "&About OK Zip";

    TbOpen => "打开", "Open";
    TbExtract => "解压", "Extract";
    TbNew => "新建", "New";
    TbAdd => "添加", "Add";
    TbDelete => "删除", "Delete";
    TbTest => "测试", "Test";
    TbView => "查看", "View";

    ColName => "名称", "Name";
    ColPacked => "压缩后大小", "Packed";
    ColSize => "原始大小", "Size";
    ColType => "类型", "Type";
    ColModified => "修改时间", "Modified";
    ColCrc => "CRC / 属性", "CRC / Attr";

    StReady => "就绪 — 拖放压缩文件到此处打开，或拖放文件以新建压缩文件", "Ready — drop an archive here to open it, or files to compress them";
    StSummary => "{files} 个文件，{dirs} 个文件夹", "{files} files, {dirs} folders";
    StSelected => "已选择 {n} 项", "{n} selected";
    StSizes => "原始 {size} · 压缩后 {packed}", "{size} → {packed}";
    StRatio => "压缩率 {ratio}", "Ratio {ratio}";
    StEncrypted => "已加密", "encrypted";
    StSolid => "固实", "solid";
    StVolumes => "{n} 个分卷", "{n} volumes";
    FolderType => "文件夹", "Folder";
    UpRow => "..", "..";

    XTitle => "解压", "Extract";
    XTitleSel => "解压所选文件", "Extract selected files";
    XDest => "目标文件夹：", "Destination folder:";
    XBrowse => "浏览(&B)...", "&Browse...";
    XSubfolder => "解压到子文件夹 \"{name}\\\"", "Extract into sub-folder \"{name}\\\"";
    XOwGroup => "文件已存在时", "If a file already exists";
    XOwAsk => "询问", "Ask me";
    XOwOverwrite => "覆盖", "Overwrite";
    XOwSkip => "跳过", "Skip";
    XOwRename => "自动重命名", "Rename automatically";
    XOther => "其他选项", "Options";
    XOpenAfter => "完成后打开文件夹", "Open folder when done";
    XPassword => "密码（可选）：", "Password (optional):";
    XScopeAll => "将解压全部 {n} 个文件。", "All {n} files will be extracted.";
    XScopeSel => "将解压选中的 {n} 项。", "The {n} selected items will be extracted.";
    XPickFolder => "选择解压目标文件夹", "Choose the destination folder";

    CTitle => "新建压缩文件", "New archive";
    CPath => "压缩文件名：", "Archive file name:";
    COpts => "压缩选项", "Compression";
    CFormat => "格式：", "Format:";
    CLevel => "压缩级别：", "Level:";
    CSplit => "分卷大小：", "Split size:";
    CSolid => "固实压缩 (7z)", "Solid archive (7z)";
    CEnc => "加密", "Encryption";
    CPw => "密码：", "Password:";
    CPw2 => "确认密码：", "Confirm:";
    CShowPw => "显示密码", "Show password";
    CEncMethod => "加密方式：", "Method:";
    CEncNames => "加密文件名 (7z)", "Encrypt file names (7z)";
    CInfo => "将压缩 {n} 个项目：{list}", "{n} items will be compressed: {list}";
    CLevel0 => "0 - 仅存储", "0 - Store";
    CLevel1 => "1 - 最快", "1 - Fastest";
    CLevel3 => "3 - 快速", "3 - Fast";
    CLevel5 => "5 - 标准", "5 - Normal";
    CLevel7 => "7 - 最大", "7 - Maximum";
    CLevel9 => "9 - 极限", "9 - Ultra";
    CSplitNone => "不分卷", "No split";
    CPwMismatch => "两次输入的密码不一致。", "The passwords do not match.";
    CNoPwFormat => "{fmt} 格式不支持密码，请选择 ZIP 或 7Z。", "{fmt} archives cannot be encrypted. Choose ZIP or 7Z.";
    CBadSplit => "无法识别的分卷大小：{s}\n示例：100M、700MB、4G", "Unrecognised split size: {s}\nExamples: 100M, 700MB, 4G";
    CNoPath => "请输入压缩文件名。", "Please enter an archive file name.";
    CExists => "文件已存在：\n{path}\n\n是否覆盖？", "The file already exists:\n{path}\n\nOverwrite it?";

    PTitle => "输入密码", "Enter password";
    PPrompt => "“{name}” 已加密，请输入密码：", "\"{name}\" is encrypted. Enter the password:";
    PWrong => "密码错误，请重新输入“{name}”的密码：", "Wrong password. Enter the password for \"{name}\" again:";
    PShow => "显示密码", "Show password";

    GExtracting => "正在解压…", "Extracting…";
    GCompressing => "正在压缩…", "Compressing…";
    GTesting => "正在测试…", "Testing…";
    GOpening => "正在打开…", "Opening…";
    GUpdating => "正在更新压缩文件…", "Updating archive…";
    GStats => "{done} / {total}    已用 {elapsed}    剩余 {remain}", "{done} / {total}    elapsed {elapsed}    left {remain}";
    GStatsNoTotal => "{done}    已用 {elapsed}", "{done}    elapsed {elapsed}";
    GCancel => "取消", "Cancel";
    GCancelling => "正在取消…", "Cancelling…";

    KText => "目标文件已存在：\n{path}\n\n是 = 覆盖    否 = 跳过    取消 = 停止\n按住 Shift 点击“是/否”可应用到后续所有文件。", "The file already exists:\n{path}\n\nYes = overwrite    No = skip    Cancel = stop\nHold Shift while clicking Yes/No to apply to all remaining files.";

    STitle => "设置", "Settings";
    SShellGroup => "资源管理器右键菜单", "Explorer context menu";
    SShellText => "添加：用 OK Zip 打开、解压到当前文件夹、解压到 \"名称\\\"、压缩为 \"名称.zip\"、压缩为 7z。仅对当前用户生效，无需管理员权限。", "Adds: Open with OK Zip, Extract here, Extract to \"name\\\", Compress to \"name.zip\", Compress to 7z. Current user only; no admin rights needed.";
    SReg => "注册右键菜单", "Register context menu";
    SUnreg => "取消注册", "Unregister";
    SAssocGroup => "文件关联", "File associations";
    SAssoc => "关联压缩文件类型", "Associate archive types";
    SUnassoc => "取消关联", "Remove associations";
    SDefApps => "打开 Windows“默认应用”设置…", "Open Windows Default apps…";
    SMore => "更多设置（界面主题、解压默认值、编码等）将在后续版本中提供。", "More settings (theme, extraction defaults, encodings…) are coming in later versions.";
    SClose => "关闭", "Close";
    SDoneReg => "✓ 已注册右键菜单。", "✓ Context menu registered.";
    SDoneUnreg => "✓ 已移除右键菜单。", "✓ Context menu removed.";
    SDoneAssoc => "✓ 已关联。Windows 10/11 可能需要在“默认应用”中确认。", "✓ Associated. Windows 10/11 may ask you to confirm in Default apps.";
    SDoneUnassoc => "✓ 已取消关联。", "✓ Associations removed.";
    SFailed => "操作失败：{err}", "Failed: {err}";

    VOpen => "用 OK Zip 打开", "Open with OK Zip";
    VExtractHere => "解压到当前文件夹", "Extract here";
    VExtractTo => "解压到 \"名称\\\"", "Extract to \"name\\\"";
    VCompressZip => "压缩为 \"名称.zip\"", "Compress to \"name.zip\"";
    VCompress7z => "压缩为 \"名称.7z\"", "Compress to \"name.7z\"";
    VAssocDesc => "OK Zip 压缩文件", "OK Zip archive";

    MExtractDone => "解压完成：{n} 个文件 → {dest}", "Extracted {n} files → {dest}";
    MTestOk => "测试通过：{n} 个文件（{size}），未发现错误。", "Test passed: {n} files ({size}), no errors found.";
    MTestFail => "发现 {n} 个错误：\n\n{list}", "{n} errors found:\n\n{list}";
    MDeleteConfirm => "确定要从压缩文件中删除选中的 {n} 项吗？\n此操作无法撤销。", "Delete the {n} selected items from the archive?\nThis cannot be undone.";
    MReadOnly => "{fmt} 格式的压缩文件暂不支持修改。", "{fmt} archives cannot be modified.";
    MSplitReadOnly => "分卷压缩文件暂不支持修改。", "Split (multi-volume) archives cannot be modified.";
    MNoArchive => "请先打开一个压缩文件。", "Open an archive first.";
    MNoSelection => "请先选择文件。", "Select a file first.";
    MAddToArchiveQ => "将 {n} 个项目添加到当前压缩文件？\n\n是 = 添加到当前压缩文件\n否 = 新建压缩文件", "Add {n} items to the open archive?\n\nYes = add to this archive\nNo = create a new archive";
    MAbout => "OK Zip {ver}\n\n仿 Bandizip 的原生 Windows 压缩软件，使用 Rust 编写。\n\n可读取：{read}\n可创建：{write}\n\n压缩引擎：okzip-core（可在 macOS 等平台复用）", "OK Zip {ver}\n\nA Bandizip-style native Windows archiver written in Rust.\n\nReads: {read}\nCreates: {write}\n\nEngine: okzip-core (portable to macOS and beyond)";
    MOpenFilter => "压缩文件", "Archives";
    MAllFiles => "所有文件", "All files";
    MPickFiles => "选择要压缩的文件", "Choose files to compress";
    MPickAddFiles => "选择要添加到压缩文件的文件", "Choose files to add";
    MPickAddFolder => "选择要添加的文件夹", "Choose a folder to add";
    MSaveAs => "保存压缩文件为", "Save archive as";
    MProps => "文件：{path}\n格式：{fmt}{flags}\n分卷：{vols}\n大小：{asize}\n\n{files} 个文件，{dirs} 个文件夹\n原始大小：{size}\n压缩率：{ratio}", "File: {path}\nFormat: {fmt}{flags}\nVolumes: {vols}\nSize: {asize}\n\n{files} files, {dirs} folders\nOriginal size: {size}\nRatio: {ratio}";

    EPasswordRequired => "需要密码。", "A password is required.";
    EWrongPassword => "密码错误。", "Wrong password.";
    ECancelled => "操作已取消。", "The operation was cancelled.";
    EUnknownFormat => "无法识别的压缩文件格式。", "Unrecognised archive format.";
    EUnsupported => "不支持：{0}", "Not supported: {0}";
    ECorrupt => "压缩文件已损坏：{0}", "The archive is damaged: {0}";
    EIo => "读写错误：{0}", "I/O error: {0}";
}

/// Localised description of a core error.
pub fn error_text(e: &okzip_core::Error) -> String {
    use okzip_core::Error as E;
    match e {
        E::PasswordRequired => tr(S::EPasswordRequired).into(),
        E::WrongPassword => tr(S::EWrongPassword).into(),
        E::Cancelled => tr(S::ECancelled).into(),
        E::UnknownFormat => tr(S::EUnknownFormat).into(),
        E::Unsupported(m) => trf(S::EUnsupported, &[("0", m)]),
        E::Corrupt(m) => trf(S::ECorrupt, &[("0", m)]),
        E::Io(io) => trf(S::EIo, &[("0", &io.to_string())]),
        E::Other(m) => m.clone(),
    }
}
