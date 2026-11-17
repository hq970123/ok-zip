# OK Zip

<img src="app/res/okzip-128.png" width="64" align="right" alt="OK Zip icon">

**OK Zip** 是一款仿 Bandizip 的 Windows 压缩软件，使用 Rust 编写，界面采用**原生 Win32 通用控件**（无 WebView、无 egui）。
压缩引擎 `okzip-core` 与界面完全分离、跨平台，后续的 macOS（AppKit）版本将直接复用它。

> 当前版本：v0.2（预览）

## 功能

### 格式支持

| 格式 | 打开 / 列表 / 解压 / 测试 | 新建 | 密码 |
|------|:---:|:---:|------|
| ZIP | ✅ | ✅ Deflate 0–9 级 | AES-256（默认）/ ZipCrypto；可读取两者 |
| 7Z | ✅ | ✅ LZMA2 0–9 级、固实/非固实 | AES-256，可加密文件名 |
| RAR（RAR 1.3–7，含 RAR4/RAR5） | ✅（纯 Rust `rars`） | — | 可读取 AES 加密文件及加密文件名（头部加密） |
| TAR | ✅ | ✅ | — |
| TAR.GZ / TGZ | ✅ | ✅ | — |
| TAR.BZ2 | ✅ | ✅ | — |
| TAR.XZ / TXZ | ✅ | ✅ | — |
| TAR.ZST / TZST | ✅ | ✅ | — |
| GZ / BZ2 / XZ / ZST（单文件） | ✅ | — | — |

- **分卷**：可读取 `*.001/.002…` 分卷（ZIP、7Z 等）以及 RAR 分卷（`x.part1.rar…` 新命名与 `x.rar/x.r00…` 旧命名，从任意一卷打开均可）；新建 ZIP / 7Z / TAR 系列时可按大小分卷（7-Zip/Bandizip 兼容的字节切分方式）。
- **中文文件名**：非 UTF-8 的 ZIP 文件名自动按 GBK 解码。
- **安全解压**：自动过滤 `..`、绝对路径、盘符和非法字符（防 Zip Slip）。
- **测试压缩包**：逐项解码并校验 CRC，报告损坏文件。
- **编辑压缩包**：向 ZIP / 7Z / TAR 系列添加、删除文件（ZIP 原样复制未改动的条目，不重新压缩）。
- 进度回调 + 随时取消；文件已存在时可选「询问 / 覆盖 / 跳过 / 自动重命名」，询问时回调可拿到现有文件与压缩包内文件的大小和修改时间。

### 界面（Windows）

- Bandizip 风格主窗口：菜单栏（文件 / 编辑 / 查看 / 工具 / 帮助）、大图标工具栏（打开、解压、新建、添加、删除、测试、查看，每个按钮使用独立绘制的内嵌图标）、左侧文件夹树、右侧详细信息列表（名称、压缩后大小、原始大小、类型、修改时间、CRC、属性），文件图标与类型名来自系统 Shell；空文件夹的两个大小列留空。
- 状态栏显示文件数、大小、压缩率、格式/加密/固实/分卷信息。
- 拖放：把压缩包拖进窗口即打开；拖入普通文件即新建压缩包（或添加到当前压缩包）。
- **拖出解压**：在列表中选中文件/文件夹，直接拖到资源管理器或桌面即可解压到该处（先解压到临时目录，再通过自实现的 `IDataObject`/`IDropSource` 以 `CF_HDROP` 交给资源管理器）。
- 双击文件：解压到临时目录并用关联程序打开；双击文件夹进入；`..` 返回上级。
- 对话框：「解压到…」（目标文件夹、子文件夹、覆盖策略、密码、完成后打开文件夹）、「新建压缩文件」（格式、级别、分卷大小、密码及确认、加密方式、加密文件名、固实）、密码输入框、带取消按钮的进度对话框（后台线程执行）。
- **文件替换确认**：解压时遇到同名文件，弹出对话框显示文件名、现有文件与新文件的大小和修改时间（标出较新的一方），按钮「覆盖 / 跳过 / 自动重命名 / 取消」，可勾选「对后续所有文件执行相同操作」。后台线程阻塞等待，由界面线程弹出对话框。
- **设置**（工具 → 设置，选项卡式属性表，点「确定」后保存到 `%APPDATA%\OKZip\settings.json`）：
  - 常规：界面语言（自动 / 中文 / English）、记住窗口大小/位置/列宽/分隔条、清除最近打开列表；
  - 解压：工具栏「解压」按钮的默认动作（解压到当前文件夹 / 解压到同名文件夹（智能）/ 每次询问）、文件已存在时的默认处理、解压后打开文件夹；
  - 压缩：新建压缩文件的默认格式与压缩级别（右键「压缩为 ZIP / 7Z」也使用该级别）；
  - 右键菜单与关联：注册/取消资源管理器右键菜单、关联压缩文件类型、打开 Windows「默认应用」。
- 文件 → 最近打开：最近 10 个压缩文件。
- **资源管理器右键菜单**（仅当前用户，HKCU，无需管理员，无 Shell 扩展 DLL）：通过 `ExtendedSubCommandsKey` 显示为一个「OK Zip」级联子菜单——压缩文件上为「打开、解压到当前文件夹、解压到同名文件夹、测试、压缩为 ZIP、压缩为 7Z」，其他文件和文件夹上为「压缩为 ZIP、压缩为 7Z」。多选文件压缩时，资源管理器为每个文件启动一个进程，OK Zip 用命名互斥量 + 临时请求文件把它们汇总到第一个进程，最终只生成**一个**压缩包。
- 简体中文界面（「自动」时系统语言为中文用中文，否则英文；环境变量 `OKZIP_LANG=zh|en` 优先）。
- 字体：系统装有 Microsoft YaHei UI（Windows 8 及以上默认自带）时，中英文界面都使用它，英文界面中的中文文件名也不会再混用日文字形；否则使用 Segoe UI。
- 每显示器 DPI 感知（PerMonitorV2）、通用控件 v6、内嵌图标与版本信息。

### 命令行

GUI 程序 `okzip.exe`：

```text
okzip.exe <压缩包>                       打开压缩包
okzip.exe x  <压缩包>                    解压到当前文件夹
okzip.exe xs <压缩包>                    智能解压到 "名称\"（压缩包内已有同名根目录时不再套一层）
okzip.exe t  <压缩包>                    测试
okzip.exe a [--format zip|7z|tgz|…] [--quick] [--gather] [--out 文件] <文件/文件夹…>
                                         把所有路径压缩为一个压缩包（--quick 直接按设置中的默认
                                         格式/级别压缩，否则弹出对话框；--gather 供右键菜单使用，
                                         汇总多选时同时启动的多个进程）
okzip.exe --register-shell / --unregister-shell / --associate / --unassociate
```

命令行工具 `okzip-cli`（跨平台，便于测试引擎）：

```text
okzip-cli list    <archive> [-p 密码]
okzip-cli extract <archive> [-o 目录] [-p 密码] [--skip|--rename] [--smart] [--only 路径]...
okzip-cli create  <archive> <文件...> [-f zip|7z|tar|tgz|tbz2|txz|tzst] [-l 0-9] [-p 密码]
                  [--zipcrypto] [--encrypt-headers] [--no-solid] [--split 100M]
okzip-cli test    <archive> [-p 密码]
okzip-cli add     <archive> <文件...> [--into 目录]
okzip-cli delete  <archive> <条目路径...>
okzip-cli formats
```

## 项目结构

```text
ok-zip/
├─ crates/okzip-core/     跨平台压缩引擎（格式识别、列表/解压/测试/新建/编辑、分卷、进度与取消）
│  └─ tests/roundtrip.rs  每种可写格式的往返测试（含密码、分卷、编辑、取消、损坏检测）
├─ crates/okzip-cli/      命令行前端
├─ app/                   Windows 原生界面（windows-sys + Win32 通用控件）
│  ├─ res/                app.rc（对话框模板、图标、清单、版本信息）、app.manifest、okzip.ico
│  └─ src/win/            主窗口、对话框、进度、注册表、多语言字符串
├─ installer/okzip.iss    Inno Setup 安装脚本
└─ .github/workflows/     CI：Linux 测试引擎；Windows 构建、打包便携版 + 安装包并发布 Release
```

## 构建

需要 Rust stable（1.88+）。

```bash
# 引擎测试（Linux / macOS / Windows 均可）
cargo test -p okzip-core

# 命令行工具
cargo build -p okzip-cli --release

# Windows 图形界面（在 Windows 上，MSVC 工具链）
cargo build -p okzip --release
# 产物：target/release/okzip.exe
```

- RAR 解压使用纯 Rust 的 [`rars`](https://github.com/bitplane/rars) crate（Apache-2.0），不再需要 C++ 编译器，
  也不依赖 UnRAR。RAR 默认开启（`rar` feature），不需要时可关闭：`cargo build -p okzip --no-default-features`。
- 在 Linux 上交叉编译（可完整链接）Windows 界面：
  ```bash
  rustup target add x86_64-pc-windows-gnu
  sudo apt-get install mingw-w64
  cargo build --target x86_64-pc-windows-gnu -p okzip
  ```
  正式发布仍使用 MSVC 工具链（见 CI）。
- RAR 读取测试使用 `crates/okzip-core/tests/data/rar/` 中的真实 RAR 样本（来源与许可见该目录的 `README.md`）。

### CI / 发布

推送到 `main` 或推送任意 tag 时，GitHub Actions 会：

1. 在 Ubuntu 上运行 `okzip-core` 全部测试（含 RAR 样本测试），并以 `x86_64-pc-windows-gnu` 交叉编译 Windows 界面做链接检查；
2. 在 `windows-latest` 上运行 `okzip-core` 测试，并以 `x86_64-pc-windows-msvc` 构建 Release；
3. 打包 `okzip-portable-x64.zip`（便携版）与 `okzip-setup-x64.exe`（Inno Setup 安装包，英文 / 简体中文向导，按用户安装，可选右键菜单与文件关联；版本号 `0.2.<运行编号>`）；
4. 发布 GitHub Release，标签为 `build-<运行编号>`。

另有 `screenshots` 工作流：在 `windows-latest` 上运行 `okzip.exe` 做界面冒烟测试（主窗口、各对话框、设置的四个选项卡、文件替换对话框、设置持久化、右键菜单注册/注销、多选汇总压缩），并上传 PNG 截图产物 `screenshots`。

安装包的简体中文翻译 `installer/ChineseSimplified.isl` 取自 Inno Setup 官方仓库（Inno Setup License，见 `installer/ChineseSimplified.LICENSE.txt`）。

## 路线图

已完成（v0.2）：
- [x] 设置持久化（JSON）：默认解压目标、覆盖策略、解压后打开文件夹、默认格式与级别、窗口大小/位置/列宽/分隔条、界面语言、最近打开
- [x] 选项卡式设置（常规 / 解压 / 压缩 / 右键菜单与关联）
- [x] 真正的文件替换对话框（大小 + 修改时间，覆盖 / 跳过 / 自动重命名 / 取消，应用到全部）
- [x] 从列表拖出文件到资源管理器（IDataObject / IDropSource）
- [x] 级联右键菜单「OK Zip」（注册表 ExtendedSubCommandsKey，无需 DLL），多选文件一次压缩为一个压缩包
- [x] 安装包简体中文界面，可选右键菜单与文件关联
- [x] 独立的工具栏图标、CRC 与属性分列、YaHei UI 字体

计划中：
- [ ] 解压/压缩更多格式：ISO、CAB、LZH、ZIPX(LZMA/PPMd 写入)、WIM 等
- [ ] 右键菜单显示真实名称「解压到“xxx\”」与图标（需要 IExplorerCommand / Shell 扩展 DLL）
- [ ] 拖出时延迟渲染（拖放完成后再解压，大文件不必先等待）
- [ ] 原地重命名、新建文件夹、注释编辑、压缩包内搜索
- [ ] 解压后自动删除压缩包、按文件夹分别压缩、自解压（SFX）
- [ ] 代码页选择（Shift-JIS / Big5 等）与文件名预览
- [ ] 界面主题 / 深色模式
- [ ] 更精细的进度（TAR 解压按原始大小）与多线程压缩
- [ ] 新建 RAR：`rars` 已支持写入 RAR（含加密、分卷、恢复记录），后续可在 OK Zip 中开放 RAR 创建（目前 RAR 仅支持读取）
- [ ] 7z 多卷原生写入的实时分卷（目前为写完后切分，需要额外一份磁盘空间）
- [ ] **macOS 版**：基于 AppKit（`objc2`）的原生界面，复用 `okzip-core`；Finder 快速操作 / 服务菜单
- [ ] 代码签名与自动更新

## 许可

MIT（见 `LICENSE`）。

第三方组件：

- RAR 解压使用 [`rars`](https://github.com/bitplane/rars)（Apache License 2.0），为 RAR 格式的独立 Rust 实现，不包含 UnRAR 源码，因此不再受 UnRAR 许可限制。
- 其余依赖（`zip`、`sevenz-rust2`、`tar`、`flate2`、`bzip2`、`liblzma`、`zstd` 等）均为 MIT / Apache-2.0 / BSD 类许可。
- 测试样本：`ssokolow/rar-test-files`（CC0 1.0）与 `rars` 测试用例（Apache-2.0），仅用于测试，不随程序发布。
