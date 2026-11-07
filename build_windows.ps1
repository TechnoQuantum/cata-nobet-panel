# Builds dist\NobetPanel.exe (single file, no runtime dependencies).
#
# Requirements: Rust (rustup, stable) and either
#   * the MSVC toolchain (Visual Studio Build Tools), or
#   * the GNU toolchain plus a MinGW-w64 bin folder on PATH (dlltool, as, windres),
#     e.g. a portable WinLibs build. Pass its folder with -MinGW.
param(
    [string]$MinGW = "$env:USERPROFILE\winlibs\mingw64\bin",
    [switch]$SkipTests
)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$cargo = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargo) { $env:PATH = "$cargo;$env:PATH" }
if ((rustc -vV | Select-String "host:").ToString() -match "gnu") {
    if (-not (Test-Path (Join-Path $MinGW "dlltool.exe"))) {
        throw "GNU toolchain needs MinGW-w64 binutils. Pass -MinGW <folder containing dlltool.exe, as.exe, windres.exe>."
    }
    $env:PATH = "$MinGW;$env:PATH"
}

# cargo reports progress on stderr, which Windows PowerShell treats as an
# error under "Stop"; rely on the exit codes instead.
$ErrorActionPreference = "Continue"
if (-not $SkipTests) {
    cargo test --release
    if ($LASTEXITCODE -ne 0) { throw "Tests failed" }
}
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Build failed" }
$ErrorActionPreference = "Stop"

New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\NobetPanel.exe dist\NobetPanel.exe -Force
Write-Host "Built dist\NobetPanel.exe"

# Installable package: exe + install/uninstall scripts + docs and licenses.
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$pkg = "dist\NobetPanel-$version"
Remove-Item $pkg -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $pkg | Out-Null
Copy-Item dist\NobetPanel.exe, installer\install.ps1, installer\uninstall.ps1, installer\Kur.cmd, README.md, README_TR.md, CHANGELOG.md, THIRD_PARTY_NOTICES.md $pkg
Copy-Item licenses $pkg -Recurse
Compress-Archive -Path "$pkg\*" -DestinationPath "$pkg.zip" -Force
Write-Host "Packaged $pkg.zip"

# Single-file setup built with Windows' own IExpress: extracts to a temp folder
# and runs install.ps1 (which asks for administrator rights). IExpress has no
# subfolders, so the license texts travel flat.
$stage = Join-Path $env:TEMP "NobetPanelSetup"
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item "$pkg\*" $stage -Exclude licenses, Kur.cmd
Copy-Item licenses\* $stage
$files = Get-ChildItem $stage -File | ForEach-Object { $_.Name }
$setup = (Resolve-Path dist).Path + "\NobetPanel-$version-Setup.exe"
$tmpSetup = Join-Path $env:TEMP "NobetPanel-Setup.exe"   # IExpress dislikes spaces in paths
$i = 0
$names = ($files | ForEach-Object { "FILE$i=`"$_`""; $script:i++ }) -join "`r`n"
$i = 0
$refs = ($files | ForEach-Object { "%FILE$i%="; $script:i++ }) -join "`r`n"
$sed = @"
[Version]
Class=IEXPRESS
SEDVersion=3
[Options]
PackagePurpose=InstallApp
ShowInstallProgramWindow=0
HideExtractAnimation=1
UseLongFileName=1
InsideCompressed=0
CAB_FixedSize=0
CAB_ResvCodeSigning=0
RebootMode=N
InstallPrompt=%InstallPrompt%
DisplayLicense=%DisplayLicense%
FinishMessage=%FinishMessage%
TargetName=%TargetName%
FriendlyName=%FriendlyName%
AppLaunched=%AppLaunched%
PostInstallCmd=%PostInstallCmd%
AdminQuietInstCmd=%AdminQuietInstCmd%
UserQuietInstCmd=%UserQuietInstCmd%
SourceFiles=SourceFiles
[Strings]
InstallPrompt=
DisplayLicense=
FinishMessage=
TargetName=$tmpSetup
FriendlyName=Nobetci Eczane Paneli $version
AppLaunched=powershell.exe -NoProfile -ExecutionPolicy Bypass -File install.ps1
PostInstallCmd=<None>
AdminQuietInstCmd=
UserQuietInstCmd=
$names
[SourceFiles]
SourceFiles0=$stage\
[SourceFiles0]
$refs
"@
$sedPath = Join-Path $env:TEMP "NobetPanelSetup.sed"   # no ".." - IExpress rejects it
[IO.File]::WriteAllText($sedPath, $sed, [Text.Encoding]::ASCII)
Remove-Item $setup, $tmpSetup -ErrorAction SilentlyContinue
Start-Process "$env:SystemRoot\System32\iexpress.exe" -ArgumentList "/N", "/Q", $sedPath -Wait   # unquoted: IExpress mis-parses quotes
if (Test-Path $tmpSetup) { Move-Item $tmpSetup $setup -Force }
if (-not (Test-Path $setup)) { throw "IExpress failed to build $setup" }
Write-Host "Packaged $setup"
