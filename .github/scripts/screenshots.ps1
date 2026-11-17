# GUI smoke test: launch okzip.exe on a sample archive, open the main dialogs
# by posting WM_COMMAND with the menu IDs from app/src/win/ids.rs, and save
# PNG screenshots (screen copy + PrintWindow of the HWND) to $Out.
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
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
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
    foreach ($vk in 0x28, 0x6B, 0x28) {   # Down, Numpad+ (expand), Down
      [W]::PostMessageW($tree, 0x0100, [IntPtr]$vk, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 400
    }
    [W]::PostMessageW($tree, 0x0100, [IntPtr]0x26, [IntPtr]::Zero) | Out-Null   # back Up to 示例项目
    Start-Sleep -Seconds 1
    Shot $main "01b-main-zh-folder"
  } else { Write-Warning "tree view not found" }

  [W]::PostMessageW($main, $WM_COMMAND, [IntPtr]40002, [IntPtr]::Zero) | Out-Null   # ID_EXTRACT
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  Shot $dlg "03-extract-dialog-zh"; CloseDialog $dlg

  [W]::PostMessageW($main, $WM_COMMAND, [IntPtr]40017, [IntPtr]::Zero) | Out-Null   # ID_SETTINGS
  $dlg = Dialog $p; Start-Sleep -Milliseconds 800
  Shot $dlg "04-settings-dialog-zh"; CloseDialog $dlg

  if ($p.HasExited) { throw "SMOKE TEST FAILED: okzip.exe died after closing dialogs (code $($p.ExitCode))" }
  Stop-Process -Id $p.Id -Force

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
  Stop-Process -Id $p.Id -Force
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
