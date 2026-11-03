# Nöbetçi Eczane Paneli 2.x kurulumu.
#
# - Programı C:\Program Files\NobetPanel klasörüne kopyalar.
# - Verileri C:\ProgramData\NobetPanel klasörüne taşır (servis ve pencere ortak kullanır).
# - "NobetPanel" Windows servisini kurar: Windows açılınca, oturum açılmasa da
#   paneli günceller; çökerse kendiliğinden yeniden başlar.
# - Oturum açılınca pencereyi simge durumunda başlatır, Başlat menüsü ve
#   masaüstüne kısayol koyar.
# - 1.x (Python) kurulumunun başlangıç kaydını ve eski kopyaları kaldırır.
#
# Kullanım (yönetici izni ister):  powershell -ExecutionPolicy Bypass -File install.ps1

param(
    [string]$Source = (Join-Path $PSScriptRoot "NobetPanel.exe"),
    [string[]]$OldCopies = @()
)

$ErrorActionPreference = "Stop"

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $argList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Source", "`"$Source`"")
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argList -Wait
    exit
}

$ServiceName = "NobetPanel"
$InstallDir = Join-Path $env:ProgramFiles "NobetPanel"
$Exe = Join-Path $InstallDir "NobetPanel.exe"
$DataDir = Join-Path $env:ProgramData "NobetPanel"
$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"

if (-not (Test-Path $Source)) { throw "NobetPanel.exe bulunamadı: $Source" }

Write-Host "1/6 Çalışan kopyalar durduruluyor..."
if (Get-Service $ServiceName -ErrorAction SilentlyContinue) {
    Stop-Service $ServiceName -Force -ErrorAction SilentlyContinue
}
Get-Process NobetPanel -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

Write-Host "2/6 Program kopyalanıyor: $InstallDir"
New-Item -ItemType Directory -Force $InstallDir | Out-Null
Copy-Item $Source $Exe -Force
foreach ($extra in "README.md", "README_TR.md", "THIRD_PARTY_NOTICES.md", "licenses", "uninstall.ps1") {
    $p = Join-Path $PSScriptRoot $extra
    if (Test-Path $p) { Copy-Item $p $InstallDir -Recurse -Force }
}

Write-Host "3/6 Veriler: $DataDir"
New-Item -ItemType Directory -Force $DataDir | Out-Null
# Users may modify the shared data (the window runs as the signed-in user).
& icacls $DataDir /grant "*S-1-5-32-545:(OI)(CI)M" /T /Q | Out-Null
$shared = Join-Path $DataDir "nobet_listesi.json"
$userData = Join-Path $env:LOCALAPPDATA "NobetPanel\nobet_listesi.json"
if (-not (Test-Path $shared) -and (Test-Path $userData)) {
    Copy-Item $userData $shared
    Write-Host "    Mevcut nöbet listesi ve ayarlar taşındı ($userData)."
}

Write-Host "4/6 Eski kurulum kaldırılıyor..."
Remove-ItemProperty -Path $RunKey -Name "NobetPanel" -ErrorAction SilentlyContinue
$startup = [Environment]::GetFolderPath("Startup")
Get-ChildItem $startup -Filter "*Nöbet*" -ErrorAction SilentlyContinue | Remove-Item -Force
Get-ChildItem $startup -Filter "*Nobet*" -ErrorAction SilentlyContinue | Remove-Item -Force
foreach ($old in $OldCopies) {
    if (Test-Path $old) { Remove-Item $old -Force; Write-Host "    Silindi: $old" }
}

Write-Host "5/6 Windows servisi kuruluyor..."
$bin = "`"$Exe`" --service"
if (Get-Service $ServiceName -ErrorAction SilentlyContinue) {
    & sc.exe config $ServiceName binPath= $bin start= delayed-auto | Out-Null
} else {
    New-Service -Name $ServiceName -BinaryPathName $bin -DisplayName "Nöbetçi Eczane Paneli" -StartupType Automatic | Out-Null
    & sc.exe config $ServiceName start= delayed-auto | Out-Null
}
& sc.exe description $ServiceName "Nöbet listesini Bluetooth LED panele gönderir; Windows açılınca oturum açılmasa da çalışır." | Out-Null
# Restart after 10 s, 30 s, then every 60 s if it ever stops unexpectedly.
& sc.exe failure $ServiceName reset= 86400 actions= restart/10000/restart/30000/restart/60000 | Out-Null
Start-Service $ServiceName

Write-Host "6/6 Oturum açılışında başlatma ve kısayollar..."
Set-ItemProperty -Path $RunKey -Name "NobetPanel" -Value "`"$Exe`" --minimized"
$shell = New-Object -ComObject WScript.Shell
foreach ($dir in @([Environment]::GetFolderPath("Programs"), [Environment]::GetFolderPath("Desktop"))) {
    $lnk = $shell.CreateShortcut((Join-Path $dir "Nöbetçi Eczane Paneli.lnk"))
    $lnk.TargetPath = $Exe
    $lnk.WorkingDirectory = $InstallDir
    $lnk.IconLocation = "$Exe,0"
    $lnk.Save()
}

Start-Process $Exe
Write-Host ""
Write-Host "Kurulum tamamlandı. Servis durumu: $((Get-Service $ServiceName).Status)"
