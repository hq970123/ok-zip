# OK Zip

<img src="app/res/okzip-128.png" width="64" align="right" alt="OK Zip icon">

**OK Zip** 是一款仿 Bandizip 的 Windows 压缩软件，使用 Rust 编写，界面采用**原生 Win32 通用控件**（无 WebView、无 egui）。
压缩引擎 `okzip-core` 与界面完全分离、跨平台，后续的 macOS（AppKit）版本将直接复用它。

> 当前版本：v0.1（早期预览）

## 功能

### 格式支持

| 格式 | 打开 / 列表 / 解压 / 测试 | 新建 | 密码 |
|------|:---:|:---:|------|
| ZIP | ✅ | ✅ Deflate 0–9 级 | AES-256（默认）/ ZipCrypto；可读取两者 |
| 7Z | ✅ | ✅ LZMA2 0–9 级、固实/非固实 | AES-256，可加密文件名 |
| RAR（RAR4/RAR5） | ✅（UnRAR） | — | 可读取加密包 |
| TAR | ✅ | ✅ | — |
| TAR.GZ / TGZ | ✅ | ✅ | — |
| TAR.BZ2 | ✅ | ✅ | — |
| TAR.XZ / TXZ | ✅ | ✅ | — |
| TAR.ZST / TZST | ✅ | ✅ | — |
| GZ / BZ2 / XZ / ZST（单文件） | ✅ | — | — |

- **分卷**：可读取 `*.001/.002…` 分卷（ZIP、7Z 等）；新建 ZIP / 7Z / TAR 系列时可按大小分卷（7-Zip/Bandizip 兼容的字节切分方式）。
- **中文文件名**：非 UTF-8 的 ZIP 文件名自动按 GBK 解码。
- **安全解压**：自动过滤 `..`、绝对路径、盘符和非法字符（防 Zip Slip）。
- **测试压缩包**：逐项解码并校验 CRC，报告损坏文件。
- **编辑压缩包**：向 ZIP / 7Z / TAR 系列添加、删除文件（ZIP 原样复制未改动的条目，不重新压缩）。
- 进度回调 + 随时取消；文件已存在时可选「询问 / 覆盖 / 跳过 / 自动重命名」。

### 界面（Windows）

- Bandizip 风格主窗口：菜单栏（文件 / 编辑 / 查看 / 工具 / 帮助）、大图标工具栏（打开、解压、新建、添加、删除、测试、查看）、左侧文件夹树、右侧详细信息列表（名称、压缩后大小、原始大小、类型、修改时间、CRC/属性），图标与类型名来自系统 Shell。
- 状态栏显示文件数、大小、压缩率、格式/加密/固实/分卷信息。
- 拖放：把压缩包拖进窗口即打开；拖入普通文件即新建压缩包（或添加到当前压缩包）。
- 双击文件：解压到临时目录并用关联程序打开；双击文件夹进入；`..` 返回上级。
- 对话框：「解压到…」（目标文件夹、子文件夹、覆盖策略、密码、完成后打开文件夹）、「新建压缩文件」（格式、级别、分卷大小、密码及确认、加密方式、加密文件名、固实）、密码输入框、带取消按钮的进度对话框（后台线程执行）。
- 设置：一键注册/取消资源管理器右键菜单（仅当前用户，HKCU，无需管理员）：
  「用 OK Zip 打开」「解压到当前文件夹」「解压到 "名称\"」「压缩为 "名称.zip"」「压缩为 "名称.7z"」，以及文件关联（「打开方式」列表）。
- 简体中文界面（系统语言非中文时自动使用英文；也可用环境变量 `OKZIP_LANG=zh|en` 指定）。
- 每显示器 DPI 感知（PerMonitorV2）、通用控件 v6、Segoe UI 字体、内嵌图标与版本信息。

### 命令行

GUI 程序 `okzip.exe`：

```text
okzip.exe <压缩包>                       打开压缩包
okzip.exe x  <压缩包>                    解压到当前文件夹
okzip.exe xs <压缩包>                    智能解压到 "名称\"（压缩包内已有同名根目录时不再套一层）
okzip.exe t  <压缩包>                    测试
okzip.exe a [--format zip|7z|tgz|…] [--quick] [--out 文件] <文件/文件夹…>
                                         新建压缩包（--quick 直接按默认参数压缩，否则弹出对话框）
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

- RAR 解压通过 `unrar` crate 编译 UnRAR 的 C++ 源码，需要 C++ 编译器（Windows 上为 MSVC）。不需要 RAR 时可关闭：
  `cargo build -p okzip --no-default-features`。
- 在 Linux 上交叉检查 Windows 界面代码：
  ```bash
  rustup target add x86_64-pc-windows-gnu
  sudo apt-get install mingw-w64
  cargo check --target x86_64-pc-windows-gnu -p okzip
  ```
  （MinGW 的头文件名为小写，UnRAR 引用了 `PowrProf.h`/`Wbemidl.h`，可建立同名链接后通过
  `CXXFLAGS_x86_64_pc_windows_gnu=-I<目录>` 传入；正式构建请使用 MSVC。）

### CI / 发布

推送到 `main` 或推送任意 tag 时，GitHub Actions 会：

1. 在 Ubuntu 上运行 `okzip-core` 全部测试；
2. 在 `windows-latest` 上以 `x86_64-pc-windows-msvc` 构建 Release；
3. 打包 `okzip-portable-x64.zip`（便携版）与 `okzip-setup-x64.exe`（Inno Setup 安装包，按用户安装，可选右键菜单与文件关联）；
4. 发布 GitHub Release，标签为 `build-<运行编号>`。

## 路线图

- [ ] 解压/压缩更多格式：ISO、CAB、LZH、ZIPX(LZMA/PPMd 写入)、WIM 等
- [ ] 真正的级联右键菜单（IExplorerCommand / Shell 扩展），多选文件一次压缩、显示真实名称「解压到 "xxx\"」
- [ ] 从列表拖出文件到资源管理器（IDataObject）
- [ ] 原地重命名、新建文件夹、注释编辑、压缩包内搜索
- [ ] 解压后自动删除压缩包、按文件夹分别压缩、自解压（SFX）
- [ ] 代码页选择（Shift-JIS / Big5 等）与文件名预览
- [ ] 设置持久化（默认解压位置、覆盖策略、界面主题/深色模式）
- [ ] 更精细的进度（RAR 按字节、TAR 解压按原始大小）与多线程压缩
- [ ] 7z 多卷原生写入的实时分卷（目前为写完后切分，需要额外一份磁盘空间）
- [ ] **macOS 版**：基于 AppKit（`objc2`）的原生界面，复用 `okzip-core`；Finder 快速操作 / 服务菜单
- [ ] 代码签名与自动更新

## 许可

MIT（见 `LICENSE`）。RAR 解压使用 UnRAR 源码，受 UnRAR 许可约束（仅可用于解压，不得用于重建 RAR 压缩算法）。
