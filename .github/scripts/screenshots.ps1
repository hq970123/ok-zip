# GUI smoke test: launch okzip.exe on a sample archive, open the main dialogs
# by posting WM_COMMAND with the menu IDs from app/src/win/ids.rs, and save
# PNG screenshots (screen copy + PrintWindow of the HWND) to $Out.
# Also: every settings tab, the overwrite-conflict dialog (extracting the
# sample twice into one folder with policy 询问), settings persistence, the
# Explorer context-menu registration and the multi-select "--gather" compress.
param(
  [string]$Bin = "target/x86_64-pc-windows-msvc/release",
  [string]$Out = "shots"
)
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string t);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string t);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int sz);
  public static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassNameW(h, s, 256); return s.ToString(); }
  public static string Txt(IntPtr h) { var s = new StringBuilder(512); GetWindowTextW(h, s, 512); return s.ToString(); }
  public static IntPtr[] TopWindows(uint pid) {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) list.Add(h); return true; }, IntPtr.Zero);
    return list.ToArray();
  }
  public static RECT Rect(IntPtr h) {
    RECT r;
    // DWMWA_EXTENDED_FRAME_BOUNDS = 9: visible frame without the invisible resize border
    if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out r);
    return r;
  }
}
"@
[W]::SetProcessDPIAware() | Out-Null

try { Set-DisplayResolution -Width 1920 -Height 1080 -Force -ErrorAction Stop; Write-Host "Resolution set to 1920x1080" }
catch { Write-Host "Set-DisplayResolution unavailable: $($_.Exception.Message)" }
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Host "Primary screen: $($screen.Width)x$($screen.Height)"

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$exe = (Resolve-Path (Join-Path $Bin "okzip.exe")).Path
$cli = (Resolve-Path (Join-Path $Bin "okzip-cli.exe")).Path

# ---- sample data ------------------------------------------------------------
$work = Join-Path $env:RUNNER_TEMP "okzip-shots"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
$src = Join-Path $work "示例项目"
$dirs = @("文档", "图片\2024 旅行", "src\core", "src\ui", "空文件夹")
foreach ($d in $dirs) { New-Item -ItemType Directory -Force -Path (Join-Path $src $d) | Out-Null }
Set-Content -Encoding utf8 (Join-Path $src "说明.txt") ("OK Zip 截图示例`r`n" * 40)
Set-Content -Encoding utf8 (Join-Path $src "README.md") ("# Sample`r`n" * 100)
Set-Content -Encoding utf8 (Join-Path $src "文档\会议纪要 2024-05.txt") ("会议纪要内容。" * 500)
Set-Content -Encoding utf8 (Join-Path $src "文档\report-final.docx") ("x" * 20000)
Set-Content -Encoding utf8 (Join-Path $src "src\core\lib.rs") ("pub fn main() {}`r`n" * 300)
Set-Content -Encoding utf8 (Join-Path $src "src\ui\window.rs") ("// window`r`n" * 200)
$rnd = New-Object byte[] 300000; (New-Object Random 42).NextBytes($rnd)
[IO.File]::WriteAllBytes((Join-Path $src "图片\2024 旅行\西湖.jpg"), $rnd)
[IO.File]::WriteAllBytes((Join-Path $src "图片\封面.png"), $rnd[0..90000])
# Settings file used by every okzip.exe below (no Explorer windows popping up).
$env:OKZIP_SETTINGS = Join-Path $work "settings.json"
[IO.File]::WriteAllText($env:OKZIP_SETTINGS, '{ "overwrite": "ask", "open_folder_after_extract": false, "extract_target": "ask" }')
$zip = Join-Path $work "示例压缩包.zip"
& $cli create $zip $src -f zip
if ($LASTEXITCODE -ne 0) { throw "okzip-cli create failed ($LASTEXITCODE)" }
& $cli list $zip
Write-Host "sample archive: $zip"

# ---- helpers ----------------------------------------------------------------
$WM_COMMAND = 0x0111
function Front([IntPtr]$h) {
  # Alt tap lets a background process take the foreground.
  [W]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [W]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
  [W]::ShowWindow($h, 5) | Out-Null
  [W]::BringWindowToTop($h) | Out-Null
  [W]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 700
}
function Shot([IntPtr]$h, [string]$name) {
  Front $h
  $r = [W]::Rect($h)
  $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
  Write-Host "capture '$name': hwnd=$h class=$([W]::Cls($h)) title='$([W]::Txt($h))' rect=$($r.Left),$($r.Top) ${w}x$hh"
  if ($w -le 0 -or $hh -le 0) { Write-Warning "empty rect for $name"; return }
  # 1) what is on screen in the window rect
  $bmp = New-Object System.Drawing.Bitmap $w, $hh
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size $w, $hh))
  $g.Dispose(); $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  # 2) PrintWindow (works even if the desktop is not being composited to screen)
  $wr = New-Object W+RECT; [W]::GetWindowRect($h, [ref]$wr) | Out-Null
  $pw = $wr.Right - $wr.Left; $ph = $wr.Bottom - $wr.Top
  $bmp = New-Object System.Drawing.Bitmap $pw, $ph
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc(); $ok = [W]::PrintWindow($h, $hdc, 2); $g.ReleaseHdc($hdc); $g.Dispose()
  if ($ok) { $bmp.Save((Join-Path $Out "$name.printwindow.png"), [System.Drawing.Imaging.ImageFormat]::Png) }
  $bmp.Dispose()
}
function FullScreen([string]$name) {
  $s = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $s.Width, $s.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($s.Left, $s.Top, 0, 0, $s.Size); $g.Dispose()
  $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}
function WaitWindow([System.Diagnostics.Process]$p, [scriptblock]$match, [int]$timeoutSec = 20) {
  $deadline = (Get-Date).AddSeconds($timeoutSec)
  while ((Get-Date) -lt $deadline) {
    if ($p.HasExited) { throw "okzip.exe exited unexpectedly (code $($p.ExitCode))" }
    foreach ($h in [W]::TopWindows([uint32]$p.Id)) { if (& $match $h) { return $h } }
    Start-Sleep -Milliseconds 300
  }
  $all = [W]::TopWindows([uint32]$p.Id) | ForEach-Object { "$([W]::Cls($_)) '$([W]::Txt($_))'" }
  throw "window not found within ${timeoutSec}s; visible windows: $($all -join '; ')"
}
function Dialog([System.Diagnostics.Process]$p) { WaitWindow $p { param($h) [W]::Cls($h) -eq "#32770" } }
function DialogTitled([System.Diagnostics.Process]$p, [string]$title) { WaitWindow $p { param($h) ([W]::Cls($h) -eq "#32770") -and ([W]::Txt($h) -eq $title) } }
function WaitExit([System.Diagnostics.Process]$p, [int]$sec = 30) {
  if (-not $p.WaitForExit($sec * 1000)) { throw "okzip.exe did not exit within ${sec}s" }
  return $p.ExitCode
}
$PSM_SETCURSEL = 0x0465
$BM_SETCHECK = 0x00F1
function CloseDialog([IntPtr]$h) { [W]::PostMessageW($h, $WM_COMMAND, [IntPtr]2, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 800 }
function Launch([string[]]$argv, [string]$lang) {
  $env:OKZIP_LANG = $lang
  $p = Start-Process -FilePath $exe -ArgumentList $argv -PassThru -WorkingDirectory $work
  Start-Sleep -Seconds 3
  if ($p.HasExited) { throw "SMOKE TEST FAILED: okzip.exe $($argv -join ' ') exited after launch with code $($p.ExitCode)" }
  return $p
}

$failed = $false
$procs = @()
try {
  # 1) main window (Chinese UI) with the archive open, then extract + settings dialogs
  $p = Launch @("`"$zip`"") "zh"; $procs += $p
  $main = WaitWindow $p { param($h) [W]::Cls($h) -eq "OKZipMainWindow" }
  Start-Sleep -Seconds 2
  FullScreen "00-desktop"
  Shot $main "01-main-zh"

  # select the first folder in the tree so the list shows real files
  $tree = [W]::FindWindowExW($main, [IntPtr]::Zero, "SysTreeView32", $null)
  if ($tree -ne [IntPtr]::Zero) {
    # TVM_GETNEXTITEM(TVGN_ROOT) -> TVM_GETNEXTITEM(TVGN_CHILD) -> TVM_EXPAND + TVM_SELECTITEM(TVGN_CARET)
    $root = [W]::SendMessageW($tree, 0x110A, [IntPtr]0, [IntPtr]::Zero)
    $child = [W]::SendMessageW($tree, 0x110A, [IntPtr]4, $root)
    Write-Host "tree root=$root child=$child"
    [W]::SendMessageW($tree, 0x1102, [IntPtr]2, $child) | Out-Null
    [W]::SendMessageW($tree, 0x110B, [IntPtr]9, $child) | Out-Null
    Start-Sleep -Seconds 1
    Shot $main "01b-main-zh-folder"
  } else { Write-Warning "tree view not found" }

  [W]::PostMessageW($main, $WM_COMMAND, [IntPtr]40002, [IntPtr]::Zero) | Out-Null   # ID_EXTRACT
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  Shot $dlg "03-extract-dialog-zh"; CloseDialog $dlg

  [W]::PostMessageW($main, $WM_COMMAND, [IntPtr]40017, [IntPtr]::Zero) | Out-Null   # ID_SETTINGS
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  $tabs = @("04a-settings-general-zh", "04b-settings-extract-zh", "04c-settings-compress-zh", "04d-settings-shell-zh")
  for ($i = 0; $i -lt $tabs.Count; $i++) {
    [W]::SendMessageW($dlg, $PSM_SETCURSEL, [IntPtr]$i, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 600
    Shot $dlg $tabs[$i]
  }
  CloseDialog $dlg

  if ($p.HasExited) { throw "SMOKE TEST FAILED: okzip.exe died after closing dialogs (code $($p.ExitCode))" }
  # Close normally (WM_CLOSE) so the window state and recent list get saved.
  [W]::PostMessageW($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
  $code = WaitExit $p 15
  Write-Host "main window closed, exit code $code"
  $saved = Get-Content -Raw -Encoding utf8 $env:OKZIP_SETTINGS
  Write-Host "settings.json after close:`n$saved"
  $json = $saved | ConvertFrom-Json
  if (-not $json.window) { throw "SMOKE TEST FAILED: window placement was not saved" }
  if ($json.column_widths.Count -ne 7) { throw "SMOKE TEST FAILED: expected 7 saved column widths, got $($json.column_widths.Count)" }
  if (-not ($json.recent -contains $zip)) { throw "SMOKE TEST FAILED: recent list does not contain the sample" }

  # 2) create dialog: `okzip.exe a <folder>` opens it directly
  $p = Launch @("a", "`"$src`"") "zh"; $procs += $p
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  Shot $dlg "02-create-dialog-zh"; CloseDialog $dlg
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue

  # 3) English main window for comparison
  $p = Launch @("`"$zip`"") "en"; $procs += $p
  $main = WaitWindow $p { param($h) [W]::Cls($h) -eq "OKZipMainWindow" }
  Start-Sleep -Seconds 2
  Shot $main "05-main-en"
  [W]::PostMessageW($main, $WM_COMMAND, [IntPtr]40002, [IntPtr]::Zero) | Out-Null
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  Shot $dlg "06-extract-dialog-en"; CloseDialog $dlg
  [W]::PostMessageW($main, $WM_COMMAND, [IntPtr]40017, [IntPtr]::Zero) | Out-Null   # ID_SETTINGS
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  Shot $dlg "09-settings-general-en"
  [W]::SendMessageW($dlg, $PSM_SETCURSEL, [IntPtr]3, [IntPtr]::Zero) | Out-Null
  Start-Sleep -Milliseconds 600
  Shot $dlg "09b-settings-shell-en"
  CloseDialog $dlg
  Stop-Process -Id $p.Id -Force

  # 4) overwrite conflict: extract the same archive twice into one folder (policy 询问)
  $xdir = Join-Path $work "extract-test"
  New-Item -ItemType Directory -Force -Path $xdir | Out-Null
  $xzip = Join-Path $xdir "示例压缩包.zip"
  Copy-Item $zip $xzip
  $env:OKZIP_LANG = "zh"
  $p = Start-Process -FilePath $exe -ArgumentList @("x", "`"$xzip`"") -PassThru -WorkingDirectory $work; $procs += $p
  $code = WaitExit $p 60
  if ($code -ne 0) { throw "SMOKE TEST FAILED: first extraction exited with $code" }
  if (-not (Test-Path (Join-Path $xdir "示例项目\说明.txt"))) { throw "SMOKE TEST FAILED: first extraction produced no files" }
  # make one existing file different so the sizes in the dialog differ
  Set-Content -Encoding utf8 (Join-Path $xdir "示例项目\README.md") "old"
  $env:OKZIP_LANG = "zh"
  $p = Start-Process -FilePath $exe -ArgumentList @("x", "`"$xzip`"") -PassThru -WorkingDirectory $work; $procs += $p
  $dlg = DialogTitled $p "确认文件替换"; Start-Sleep -Milliseconds 800
  Shot $dlg "07-conflict-dialog-zh"
  # 对后续所有文件执行相同操作 + 跳过
  [W]::SendMessageW([W]::GetDlgItem($dlg, 1709), $BM_SETCHECK, [IntPtr]1, [IntPtr]::Zero) | Out-Null
  [W]::PostMessageW($dlg, $WM_COMMAND, [IntPtr]1711, [IntPtr]::Zero) | Out-Null
  $code = WaitExit $p 60
  if ($code -ne 0) { throw "SMOKE TEST FAILED: second extraction (skip all) exited with $code" }
  if ((Get-Content -Raw (Join-Path $xdir "示例项目\README.md")).Trim() -ne "old") { throw "SMOKE TEST FAILED: 'skip all' overwrote a file" }
  $env:OKZIP_LANG = "en"
  $p = Start-Process -FilePath $exe -ArgumentList @("x", "`"$xzip`"") -PassThru -WorkingDirectory $work; $procs += $p
  $dlg = DialogTitled $p "Confirm file replace"; Start-Sleep -Milliseconds 800
  Shot $dlg "08-conflict-dialog-en"
  [W]::PostMessageW($dlg, $WM_COMMAND, [IntPtr]2, [IntPtr]::Zero) | Out-Null   # Cancel
  WaitExit $p 30 | Out-Null

  # 5) Explorer context menu: register, inspect, unregister
  $p = Start-Process -FilePath $exe -ArgumentList "--register-shell" -PassThru -Wait
  if ($p.ExitCode -ne 0) { throw "SMOKE TEST FAILED: --register-shell exited with $($p.ExitCode)" }
  foreach ($k in @("HKCU:\Software\Classes\Directory\shell\OKZip", "HKCU:\Software\Classes\*\shell\OKZip",
                   "HKCU:\Software\Classes\SystemFileAssociations\.zip\shell\OKZip", "HKCU:\Software\Classes\OKZip.ArchiveMenu\shell\02ExtractHere\command")) {
    if (-not (Test-Path -LiteralPath $k)) { throw "SMOKE TEST FAILED: missing registry key $k" }
    $v = Get-ItemProperty -LiteralPath $k
    Write-Host "$k : $(($v.PSObject.Properties | Where-Object { $_.Name -notlike 'PS*' } | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' | ')"
  }
  try {
    $shell = New-Object -ComObject Shell.Application
    foreach ($f in @($xzip, (Join-Path $src "说明.txt"))) {
      $item = $shell.NameSpace((Split-Path $f)).ParseName((Split-Path $f -Leaf))
      $names = @($item.Verbs() | ForEach-Object { $_.Name })
      $ok = @($names | Where-Object { $_ -match "OK Zip" }).Count
      Write-Host "context menu of $(Split-Path $f -Leaf): 'OK Zip' entries = $ok; verbs = $($names -join ' | ')"
    }
  } catch { Write-Warning "Shell.Application verb listing failed: $($_.Exception.Message)" }
  $p = Start-Process -FilePath $exe -ArgumentList "--unregister-shell" -PassThru -Wait
  if (Test-Path -LiteralPath "HKCU:\Software\Classes\Directory\shell\OKZip") { throw "SMOKE TEST FAILED: --unregister-shell left keys behind" }

  # 6) multi-selection compress: one process per file (as Explorer does) -> one archive
  $gdir = Join-Path $work "gather-test"
  New-Item -ItemType Directory -Force -Path $gdir | Out-Null
  $gfiles = @("a.txt", "b.txt", "c.txt", "d.txt") | ForEach-Object { $f = Join-Path $gdir $_; Set-Content -Encoding utf8 $f ("data " + $_); $f }
  $gp = @($gfiles | ForEach-Object { Start-Process -FilePath $exe -ArgumentList @("a", "--format", "zip", "--quick", "--gather", "`"$_`"") -PassThru })
  foreach ($q in $gp) { if (-not $q.WaitForExit(60000)) { throw "SMOKE TEST FAILED: gather process hung" } }
  $made = @(Get-ChildItem $gdir -Filter *.zip)
  Write-Host "gather: exit codes $(($gp | ForEach-Object { $_.ExitCode }) -join ','); archives: $(($made | ForEach-Object { $_.Name }) -join ', ')"
  if ($made.Count -ne 1) { throw "SMOKE TEST FAILED: expected 1 archive from a 4-file selection, got $($made.Count)" }
  $listing = & $cli list $made[0].FullName | Out-String
  Write-Host $listing
  foreach ($n in @("a.txt", "b.txt", "c.txt", "d.txt")) { if ($listing -notmatch [regex]::Escape($n)) { throw "SMOKE TEST FAILED: $n missing from the gathered archive" } }
}
catch {
  Write-Host "::error::$($_.Exception.Message)"
  try { FullScreen "99-failure-desktop" } catch {}
  $failed = $true
}
finally {
  foreach ($q in $procs) { if (-not $q.HasExited) { Stop-Process -Id $q.Id -Force -ErrorAction SilentlyContinue } }
}
Get-ChildItem $Out | ForEach-Object { Write-Host "$($_.Name)  $($_.Length) bytes" }
if ($failed) { exit 1 }
