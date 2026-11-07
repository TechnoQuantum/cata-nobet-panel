# Nöbetçi Eczane Paneli 2.x kaldırma. Veriler (C:\ProgramData\NobetPanel) korunur;
# silmek için -RemoveData kullanın.
param([switch]$RemoveData)

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $argList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"")
    if ($RemoveData) { $argList += "-RemoveData" }
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argList -Wait
    exit
}

$ServiceName = "NobetPanel"
if (Get-Service $ServiceName -ErrorAction SilentlyContinue) {
    Stop-Service $ServiceName -Force -ErrorAction SilentlyContinue
    & sc.exe delete $ServiceName | Out-Null
}
Get-Process NobetPanel -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "NobetPanel" -ErrorAction SilentlyContinue
foreach ($dir in @([Environment]::GetFolderPath("Programs"), [Environment]::GetFolderPath("Desktop"))) {
    Remove-Item (Join-Path $dir "Nöbetçi Eczane Paneli.lnk") -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 1
Remove-Item (Join-Path $env:ProgramFiles "NobetPanel") -Recurse -Force -ErrorAction SilentlyContinue
if ($RemoveData) {
    Remove-Item (Join-Path $env:ProgramData "NobetPanel") -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host "Nöbetçi Eczane Paneli kaldırıldı."
