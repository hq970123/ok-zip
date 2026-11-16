; Inno Setup script for OK Zip (per-user install by default; no admin needed).
; Build: ISCC.exe /DAppVersion=0.1.0 /DSourceDir=..\dist\okzip installer\okzip.iss

#ifndef AppVersion
  #define AppVersion "0.1.0"
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

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "contextmenu"; Description: "Add OK Zip to the Explorer right-click menu / 添加到资源管理器右键菜单"; GroupDescription: "Explorer:"
Name: "associate"; Description: "Offer OK Zip for archive files (Open with) / 关联压缩文件"; GroupDescription: "Explorer:"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\okzip.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\okzip-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\OK Zip"; Filename: "{app}\okzip.exe"
Name: "{autodesktop}\OK Zip"; Filename: "{app}\okzip.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\okzip.exe"; Parameters: "--register-shell"; Flags: runhidden; Tasks: contextmenu
Filename: "{app}\okzip.exe"; Parameters: "--associate"; Flags: runhidden; Tasks: associate
Filename: "{app}\okzip.exe"; Description: "{cm:LaunchProgram,OK Zip}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\okzip.exe"; Parameters: "--unregister-shell"; Flags: runhidden; RunOnceId: "UnregShell"
Filename: "{app}\okzip.exe"; Parameters: "--unassociate"; Flags: runhidden; RunOnceId: "Unassoc"

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\Temp\OKZip-*"
