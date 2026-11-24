# Builds OK Zip's sparse package (package with external location) for the
# Windows 11 top-level context menu, into the staged app folder $Stage:
#
#   $Stage\context-menu-ok-zip.dll   context-menu-rs v0.2.1 IExplorerCommand DLL (MIT), sha256-pinned
#   $Stage\assets\logo*.png          logos the manifest points at (external location)
#   $Stage\okzip-shellmenu.msix      signed sparse package (AppxManifest.xml only)
#   $Stage\okzip-dev.cer             public certificate, imported by the installer (self-signed builds)
#
# Signing:
#   * env OKZIP_SIGN_PFX (base64 .pfx) + OKZIP_SIGN_PASSWORD -> that certificate
#     (a CA code-signing certificate later: no other change; set
#     OKZIP_SIGN_TRUSTED=1 so no .cer is shipped/imported).
#   * otherwise a fresh self-signed "CN=OK Zip Dev" certificate per build.
#   The manifest's Publisher is always the certificate's subject.
param(
  [Parameter(Mandatory)] [string]$Stage,
  [Parameter(Mandatory)] [string]$Version,      # a.b.c.d
  [Parameter(Mandatory)] [string]$ShellMenuExe  # okzip-shellmenu.exe (prints the manifest)
)
$ErrorActionPreference = "Stop"

$DllUrl = "https://github.com/hivepaas/context-menu-rs/releases/download/v0.2.1/context-menu-windows-x86_64.dll"
$DllSha = "557b1126fec515de47870b3aaf946624ecfbf6d4ecddf732bb73c9c78f1f7bf9"

$Stage = (Resolve-Path $Stage).Path
$dllName = (& $ShellMenuExe dll).Trim()
$dll = Join-Path $Stage $dllName
Invoke-WebRequest -Uri $DllUrl -OutFile $dll -UseBasicParsing
$sha = (Get-FileHash -Algorithm SHA256 $dll).Hash.ToLowerInvariant()
if ($sha -ne $DllSha) { throw "context-menu-windows DLL checksum mismatch: $sha" }
Write-Host "DLL: $dll ($sha)"

New-Item -ItemType Directory -Force (Join-Path $Stage "assets") | Out-Null
Copy-Item "$PSScriptRoot\..\..\installer\sparse\assets\*.png" (Join-Path $Stage "assets") -Force

# --- certificate --------------------------------------------------------------
$tmp = Join-Path $env:RUNNER_TEMP "okzip-sparse"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $tmp | Out-Null
$pfx = Join-Path $tmp "sign.pfx"
if ($env:OKZIP_SIGN_PFX) {
  [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:OKZIP_SIGN_PFX))
  $pw = $env:OKZIP_SIGN_PASSWORD
  $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($pfx, $pw)
  Write-Host "Signing with provided certificate: $($cert.Subject)"
} else {
  $pw = [Guid]::NewGuid().ToString("N")
  $cert = New-SelfSignedCertificate -Type Custom -Subject "CN=OK Zip Dev" -FriendlyName "OK Zip Dev (self-signed)" `
    -KeyUsage DigitalSignature -KeyAlgorithm RSA -KeyLength 2048 -NotAfter (Get-Date).AddYears(5) `
    -CertStoreLocation "Cert:\CurrentUser\My" `
    -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
  $sec = ConvertTo-SecureString -String $pw -Force -AsPlainText
  Export-PfxCertificate -Cert $cert -FilePath $pfx -Password $sec | Out-Null
  Write-Host "Self-signed certificate: $($cert.Subject) $($cert.Thumbprint)"
}
if (-not $env:OKZIP_SIGN_TRUSTED) {
  [IO.File]::WriteAllBytes((Join-Path $Stage "okzip-dev.cer"), $cert.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert))
}

# --- manifest + pack + sign ---------------------------------------------------
$pkg = Join-Path $tmp "package"
New-Item -ItemType Directory -Force $pkg | Out-Null
$manifest = & $ShellMenuExe manifest $Version $cert.Subject | Out-String
[IO.File]::WriteAllText((Join-Path $pkg "AppxManifest.xml"), $manifest, (New-Object System.Text.UTF8Encoding($false)))
Write-Host $manifest

$kits = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64" -Directory | Sort-Object Name -Descending
$makeappx = ($kits | ForEach-Object { Join-Path $_.FullName "makeappx.exe" } | Where-Object { Test-Path $_ } | Select-Object -First 1)
$signtool = ($kits | ForEach-Object { Join-Path $_.FullName "signtool.exe" } | Where-Object { Test-Path $_ } | Select-Object -First 1)
if (-not $makeappx -or -not $signtool) { throw "Windows SDK makeappx/signtool not found" }

$msix = Join-Path $Stage "okzip-shellmenu.msix"
& $makeappx pack /d $pkg /p $msix /nv /o
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)" }
& $signtool sign /fd SHA256 /f $pfx /p $pw $msix
if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }
Write-Host "Package: $msix ($((Get-Item $msix).Length) bytes), publisher $($cert.Subject), CLSID $(& $ShellMenuExe clsid)"
