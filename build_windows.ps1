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
