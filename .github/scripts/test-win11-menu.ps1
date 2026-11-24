# CI check of the Windows 11 top-level context menu (sparse package +
# context-menu-rs IExplorerCommand DLL):
#   1. trust the self-signed certificate, Add-AppxPackage -ExternalLocation
#   2. okzip --register-shell -> menu definition written, package serves it,
#      no classic verbs left (they would show twice)
#   3. activate the DLL's COM class like Explorer does, read titles/state of
#      the entry and its sub-commands for a .zip, a .txt and a folder, and
#      Invoke "解压到当前文件夹" -> files really extracted
#   4. the DLL is loaded in a dllhost surrogate
#   5. try to open Explorer's own menu (Apps key) and screenshot it
#   6. Remove-AppxPackage -> classic verbs come back
# Hard failures exit 1; things a Server runner cannot show are warnings.
param(
  [Parameter(Mandatory)] [string]$Stage,   # app folder holding okzip.exe, the DLL, assets, msix, cer
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

[ComImport, Guid("a08ce4d0-fa25-44ab-b57c-c7b1c323e0b9"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IExplorerCommandX {
  [PreserveSig] int GetTitle(IntPtr psia, out IntPtr name);
  [PreserveSig] int GetIcon(IntPtr psia, out IntPtr icon);
  [PreserveSig] int GetToolTip(IntPtr psia, out IntPtr tip);
  [PreserveSig] int GetCanonicalName(out Guid g);
  [PreserveSig] int GetState(IntPtr psia, int okToBeSlow, out uint state);
  [PreserveSig] int Invoke(IntPtr psia, IntPtr pbc);
  [PreserveSig] int GetFlags(out uint flags);
  [PreserveSig] int EnumSubCommands(out IEnumExplorerCommandX e);
}
[ComImport, Guid("a88826f8-186f-4987-aade-ea0cef8fbfe8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IEnumExplorerCommandX {
  [PreserveSig] int Next(uint celt, [MarshalAs(UnmanagedType.Interface)] out IExplorerCommandX cmd, out uint fetched);
  [PreserveSig] int Skip(uint celt);
  [PreserveSig] int Reset();
  [PreserveSig] int Clone(out IEnumExplorerCommandX e);
}
public static class XCmd {
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)] static extern int SHCreateItemFromParsingName(string path, IntPtr pbc, ref Guid riid, out IntPtr ppv);
  [DllImport("shell32.dll")] static extern int SHCreateShellItemArrayFromShellItem(IntPtr psi, ref Guid riid, out IntPtr ppv);
  [DllImport("ole32.dll")] static extern void CoTaskMemFree(IntPtr p);
  public static IntPtr Array(string path) {
    Guid iidItem = new Guid("43826d1e-e718-42ee-bc55-a1e261c37bfe");
    Guid iidArr = new Guid("b63ea76d-1f85-456f-a19c-48159efa858b");
    IntPtr item, arr;
    int hr = SHCreateItemFromParsingName(path, IntPtr.Zero, ref iidItem, out item);
    if (hr != 0) throw new Exception("SHCreateItemFromParsingName " + hr.ToString("X"));
    hr = SHCreateShellItemArrayFromShellItem(item, ref iidArr, out arr);
    Marshal.Release(item);
    if (hr != 0) throw new Exception("SHCreateShellItemArrayFromShellItem " + hr.ToString("X"));
    return arr;
  }
  static string Str(int hr, IntPtr p) { if (hr != 0 || p == IntPtr.Zero) return "(hr " + hr.ToString("X") + ")"; var s = Marshal.PtrToStringUni(p); CoTaskMemFree(p); return s; }
  public static IExplorerCommandX Create(string clsid) {
    var t = Type.GetTypeFromCLSID(new Guid(clsid));
    return (IExplorerCommandX)Activator.CreateInstance(t);
  }
  // "title [state]" for the entry and each sub-command; state 0 = ECS_ENABLED, 2 = ECS_HIDDEN.
  // COM objects stay inside C#: PowerShell cannot pass a __ComObject back as IExplorerCommandX.
  public static List<string> Describe(string clsid, string path) { return Describe(Create(clsid), Array(path)); }
  public static int InvokeSub(string clsid, string path, string title) { return InvokeSub(Create(clsid), Array(path), title); }
  static List<string> Describe(IExplorerCommandX root, IntPtr arr) {
    var o = new List<string>();
    IntPtr p; uint st, fl;
    int hr = root.GetTitle(arr, out p); string t = Str(hr, p);
    root.GetState(arr, 1, out st); root.GetFlags(out fl);
    hr = root.GetIcon(arr, out p); string ic = Str(hr, p);
    o.Add(t + " [state " + st + ", flags " + fl + ", icon " + ic + "]");
    IEnumExplorerCommandX e;
    if (root.EnumSubCommands(out e) == 0 && e != null) {
      IExplorerCommandX c; uint got;
      while (e.Next(1, out c, out got) == 0 && got == 1) {
        hr = c.GetTitle(arr, out p); string ct = Str(hr, p);
        c.GetState(arr, 1, out st);
        o.Add("    " + ct + " [state " + st + "]");
      }
    }
    return o;
  }
  static int InvokeSub(IExplorerCommandX root, IntPtr arr, string title) {
    IEnumExplorerCommandX e; IntPtr p;
    if (root.EnumSubCommands(out e) != 0 || e == null) return -1;
    IExplorerCommandX c; uint got;
    while (e.Next(1, out c, out got) == 0 && got == 1) {
      if (Str(c.GetTitle(arr, out p), p) == title) return c.Invoke(arr, IntPtr.Zero);
    }
    return -2;
  }
}
public static class K {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@
[K]::SetProcessDPIAware() | Out-Null
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$Stage = (Resolve-Path $Stage).Path
$exe = Join-Path $Stage "okzip.exe"
$cli = Join-Path $Stage "okzip-cli.exe"
$msix = Join-Path $Stage "okzip-shellmenu.msix"
$cer = Join-Path $Stage "okzip-dev.cer"
$shellmenu = Join-Path $Stage "okzip-shellmenu.exe"
$clsid = (& $shellmenu clsid).Trim()
$pkgName = (& $shellmenu package).Trim()
$summary = New-Object System.Collections.Generic.List[string]
function Note($s) { Write-Host $s; $summary.Add($s) }
function Shot($name) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}
function Status {
  $f = Join-Path $env:RUNNER_TEMP "okzip-shell-status.txt"
  Start-Process -FilePath $exe -ArgumentList @("--shell-status", "`"$f`"") -Wait | Out-Null
  Get-Content -Raw $f
}
function ClassicVerbs {
  # verbs context-menu-rs writes carry a value "ContextMenuRs" naming the menu
  $hits = @()
  foreach ($k in @("HKCU:\Software\Classes\*\shell", "HKCU:\Software\Classes\Directory\shell", "HKCU:\Software\Classes\SystemFileAssociations\.zip\shell")) {
    if (Test-Path -LiteralPath $k) {
      Get-ChildItem -LiteralPath $k | ForEach-Object {
        $v = Get-ItemProperty -LiteralPath $_.PSPath
        if ($v.ContextMenuRs -eq "ok-zip") { $hits += $_.Name }
      }
    }
  }
  $hits
}

$os = Get-CimInstance Win32_OperatingSystem
Note "OS: $($os.Caption) build $($os.BuildNumber) (ProductType $($os.ProductType))"
Note "CLSID: $clsid, package: $pkgName"

# sample data
$work = Join-Path $env:RUNNER_TEMP "okzip-w11"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $work "src\文档") | Out-Null
Set-Content -Encoding utf8 (Join-Path $work "src\说明.txt") "hello"
Set-Content -Encoding utf8 (Join-Path $work "src\文档\a.txt") "a"
$zip = Join-Path $work "示例.zip"
& $cli create $zip (Join-Path $work "src\说明.txt") (Join-Path $work "src\文档") | Out-Null
if (-not (Test-Path $zip)) { throw "could not make the sample zip" }
$txt = Join-Path $work "src\说明.txt"
$dir = Join-Path $work "src\文档"

$failed = $false
try {
  # 0) classic menu first (no package): verbs present
  Start-Process -FilePath $exe -ArgumentList "--register-shell" -Wait | Out-Null
  $v = @(ClassicVerbs)
  Note "Without package: $(Status) ; classic verbs: $($v.Count)"
  if ($v.Count -lt 1) { throw "classic verbs missing before the package" }

  # 1) certificate + package
  if (Test-Path $cer) {
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    Note "Imported $cer into LocalMachine\TrustedPeople"
  }
  Add-AppxPackage -Path $msix -ExternalLocation $Stage
  $pkg = Get-AppxPackage $pkgName
  if (-not $pkg) { throw "Add-AppxPackage succeeded but Get-AppxPackage $pkgName is empty" }
  Note "Add-AppxPackage OK: $($pkg.PackageFullName), SignatureKind $($pkg.SignatureKind), install location $($pkg.InstallLocation)"

  # 2) register again: the package now serves the menu, classic verbs gone
  Start-Process -FilePath $exe -ArgumentList "--register-shell" -Wait | Out-Null
  $st = Status
  $v = @(ClassicVerbs)
  Note "With package: $st ; classic verbs: $($v.Count)"
  if ($st -match "package=-") { throw "context-menu-rs does not see the package serving the menu" }
  if ($v.Count -ne 0) { throw "classic verbs still present with the package registered: $($v -join ', ')" }
  $defn = Join-Path $env:LOCALAPPDATA "context-menu-rs\ok-zip.menu"
  if (-not (Test-Path $defn)) { throw "menu definition $defn missing" }
  Write-Host (Get-Content -Raw $defn)
  $ci = Get-Item -LiteralPath "HKLM:\SOFTWARE\Classes\PackagedCom\ClassIndex\{$clsid}" -ErrorAction SilentlyContinue
  Note "PackagedCom ClassIndex entry: $([bool]$ci)"

  # 3) COM, as Explorer calls it
  foreach ($p in @($zip, $txt, $dir)) {
    $lines = [XCmd]::Describe($clsid, $p)
    Note "IExplorerCommand for $(Split-Path $p -Leaf):`n$($lines -join "`n")"
  }
  $zh = (Get-Content -Raw $defn) -match "解压到当前文件夹"
  $label = if ($zh) { "解压到当前文件夹" } else { "Extract here" }
  $hr = [XCmd]::InvokeSub($clsid, $zip, $label)
  Note "Invoke '$label' on 示例.zip -> hr $hr"
  $deadline = (Get-Date).AddSeconds(30)
  while (-not (Test-Path (Join-Path $work "说明.txt")) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
  if (Test-Path (Join-Path $work "说明.txt")) { Note "Invoke extracted 说明.txt next to the archive: OK" }
  else { throw "Invoke did not extract the archive (hr $hr)" }

  # 4) which process loaded the DLL
  $hosts = Get-Process | Where-Object { try { $_.Modules | Where-Object { $_.ModuleName -like "context-menu-ok-zip.dll" } } catch { $null } }
  Note "DLL loaded in: $(($hosts | ForEach-Object { "$($_.ProcessName)($($_.Id))" }) -join ', ')"
  if (-not $hosts) { Write-Warning "context-menu-ok-zip.dll not found in any process (surrogate may have exited)" }

  # 5) Explorer's own menu, if this edition shows it
  try {
    Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
    Start-Sleep 3
    if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe; Start-Sleep 5 }
    Start-Process explorer.exe -ArgumentList "/select,`"$zip`""
    Start-Sleep 5
    $w = Get-Process explorer | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($w) { [K]::SetForegroundWindow($w.MainWindowHandle) | Out-Null }
    Start-Sleep 1
    Shot "10-explorer-selected"
    [K]::keybd_event(0x5D, 0, 0, [UIntPtr]::Zero); [K]::keybd_event(0x5D, 0, 2, [UIntPtr]::Zero)   # VK_APPS
    Start-Sleep 3
    Shot "11-win11-context-menu"
    Note "Screenshot of Explorer's menu (Apps key) saved: 11-win11-context-menu.png"
    [K]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); [K]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero)
  } catch { Write-Warning "explorer screenshot failed: $($_.Exception.Message)" }

  # 6) remove the package: classic verbs come back
  Remove-AppxPackage -Package $pkg.PackageFullName
  Start-Process -FilePath $exe -ArgumentList "--register-shell" -Wait | Out-Null
  $v = @(ClassicVerbs)
  Note "After Remove-AppxPackage: $(Status) ; classic verbs: $($v.Count)"
  if ($v.Count -lt 1) { throw "classic verbs did not come back after removing the package" }
  Start-Process -FilePath $exe -ArgumentList "--unregister-shell" -Wait | Out-Null
  if (@(ClassicVerbs).Count -ne 0) { throw "--unregister-shell left verbs" }
}
catch {
  Write-Host "::error::WIN11 MENU TEST FAILED: $($_.Exception.Message)"
  $summary.Add("FAILED: $($_.Exception.Message)")
  try { Shot "19-win11-failure" } catch {}
  $failed = $true
}
$summary | Set-Content -Encoding utf8 (Join-Path $Out "win11-menu-test.txt")
if ($env:GITHUB_STEP_SUMMARY) { ("### Windows 11 menu test`n``````n" + ($summary -join "`n") + "`n``````") | Add-Content $env:GITHUB_STEP_SUMMARY }
if ($failed) { exit 1 }
