; Inno Setup script for OK Zip (per-user install by default; no admin needed).
; Build: ISCC.exe /DAppVersion=0.2.0 /DSourceDir=..\dist\okzip installer\okzip.iss
; Languages: English + Simplified Chinese (ChineseSimplified.isl is vendored,
; see ChineseSimplified.LICENSE.txt). The wizard picks the one matching the
; Windows UI language and shows a language selector otherwise.

#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\okzip"
#endif

[Setup]
AppId={{6C1E5B0A-3F4D-4E8B-9C21-0D5A7B9E2F11}
AppName=OK Zip
AppVersion={#AppVersion}
AppVerName=OK Zip {#AppVersion}
AppPublisher=OK Zip
DefaultDirName={autopf}\OK Zip
DefaultGroupName=OK Zip
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputDir=..\dist
OutputBaseFilename=okzip-setup-x64
SetupIconFile=..\app\res\okzip.ico
UninstallDisplayIcon={app}\okzip.exe
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
ShowLanguageDialog=auto
UsePreviousLanguage=no
VersionInfoVersion={#AppVersion}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "ChineseSimplified.isl"

[CustomMessages]
english.GroupExplorer=Explorer integration:
english.TaskContextMenu=Add an "OK Zip" submenu to the Explorer right-click menu (open, extract, test, compress)
english.TaskWin11Menu=Show "OK Zip" in the Windows 11 top-level right-click menu (registers OK Zip's package; trusting its test certificate asks for administrator approval once)
english.TaskAssociate=Associate archive files (.zip, .7z, .rar, .tar, ...) with OK Zip
chinesesimplified.GroupExplorer=资源管理器集成：
chinesesimplified.TaskContextMenu=在资源管理器右键菜单中添加“OK Zip”子菜单（打开、解压、测试、压缩）
chinesesimplified.TaskWin11Menu=在 Windows 11 一级右键菜单中显示“OK Zip”（注册 OK Zip 稀疏包；首次信任测试证书需要管理员确认一次）
chinesesimplified.TaskAssociate=将压缩文件（.zip、.7z、.rar、.tar 等）关联到 OK Zip

[Tasks]
Name: "contextmenu"; Description: "{cm:TaskContextMenu}"; GroupDescription: "{cm:GroupExplorer}"
Name: "contextmenu\win11"; Description: "{cm:TaskWin11Menu}"; Check: IsWin11
Name: "associate"; Description: "{cm:TaskAssociate}"; GroupDescription: "{cm:GroupExplorer}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\okzip.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\okzip-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
; Windows 11 top-level menu: context-menu-rs IExplorerCommand DLL (MIT, loaded by a dllhost
; surrogate, so it may be in use while upgrading/uninstalling), sparse package and logos.
Source: "{#SourceDir}\context-menu-ok-zip.dll"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "{#SourceDir}\okzip-shellmenu.msix"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\assets\*.png"; DestDir: "{app}\assets"; Flags: ignoreversion
#ifexist SourceDir + "\okzip-dev.cer"
Source: "{#SourceDir}\okzip-dev.cer"; DestDir: "{app}"; Flags: ignoreversion
#endif
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\OK Zip"; Filename: "{app}\okzip.exe"
Name: "{autodesktop}\OK Zip"; Filename: "{app}\okzip.exe"; Tasks: desktopicon

[Run]
; 1) trust the CI self-signed certificate (Local Machine > Trusted People; needs elevation once).
;    Builds signed with a CA / Store certificate ship no okzip-dev.cer and skip this.
#ifexist SourceDir + "\okzip-dev.cer"
Filename: "{sys}\certutil.exe"; Parameters: "-addstore TrustedPeople ""{app}\okzip-dev.cer"""; Verb: "runas"; Flags: shellexec runhidden waituntilterminated; Tasks: contextmenu\win11; Check: NeedDevCert
#endif
; 2) register the sparse package for the user (external location = install folder)
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Get-AppxPackage OKZip.ShellMenu | Remove-AppxPackage -ErrorAction SilentlyContinue; Add-AppxPackage -Path '{app}\okzip-shellmenu.msix' -ExternalLocation '{app}'"""; Flags: runhidden runasoriginaluser waituntilterminated; Tasks: contextmenu\win11
; 3) write the menu (context-menu-rs): with the package -> Windows 11 top-level menu; else classic verbs
Filename: "{app}\okzip.exe"; Parameters: "--register-shell"; Flags: runhidden runasoriginaluser; Tasks: contextmenu
Filename: "{app}\okzip.exe"; Parameters: "--associate"; Flags: runhidden runasoriginaluser; Tasks: associate
Filename: "{app}\okzip.exe"; Description: "{cm:LaunchProgram,OK Zip}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Get-AppxPackage OKZip.ShellMenu | Remove-AppxPackage"""; Flags: runhidden; RunOnceId: "RemoveSparse"
Filename: "{app}\okzip.exe"; Parameters: "--unregister-shell"; Flags: runhidden; RunOnceId: "UnregShell"
Filename: "{app}\okzip.exe"; Parameters: "--unassociate"; Flags: runhidden; RunOnceId: "Unassoc"

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\Temp\OKZip-gather-*"
Type: filesandordirs; Name: "{localappdata}\Temp\OKZip-*"

[Code]
function IsWin11: Boolean;
var
  V: TWindowsVersion;
begin
  GetWindowsVersionEx(V);
  Result := (V.Major = 10) and (V.Build >= 22000);
end;

{ The certificate is already trusted when an earlier install imported it. }
function NeedDevCert: Boolean;
var
  ResultCode: Integer;
begin
  Result := True;
  if Exec(ExpandConstant('{sys}\certutil.exe'), '-verifystore TrustedPeople "OK Zip Dev"', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    Result := ResultCode <> 0;
end;

{ Upgrading: unregister the old package first so Explorer's surrogate lets go of the DLL. }
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssInstall then
    Exec('powershell.exe', '-NoProfile -ExecutionPolicy Bypass -Command "Get-AppxPackage OKZip.ShellMenu | Remove-AppxPackage"', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;
