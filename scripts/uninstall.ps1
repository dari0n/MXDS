#requires -Version 5.1
<#
.SYNOPSIS
    Удаление MXDS.
.DESCRIPTION
    Цикл 16 (Release). Останавливает работающий экземпляр и движок,
    снимает автозапуск из Планировщика, удаляет осиротевшие службы
    WinDivert (восстановление после сбоя), затем удаляет каталог
    установки. -KeepData сохраняет config\config.json и logs\.
.EXAMPLE
    powershell -File uninstall.ps1
    powershell -File uninstall.ps1 -KeepData
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\MXDS"),
    [switch]$KeepData
)

$ErrorActionPreference = "Continue"
$exe = Join-Path $InstallDir "mxds.exe"

Write-Host "== Удаление MXDS из $InstallDir =="

# --- остановка работающего экземпляра (best effort) ---
if (Test-Path $exe) {
    & $exe stop | Out-Null
    Write-Host "Экземпляр: команда остановки отправлена"
}

# --- автозапуск ---
$task = Get-ScheduledTask -TaskName "MXDS" -ErrorAction SilentlyContinue
if ($task) {
    Unregister-ScheduledTask -TaskName "MXDS" -Confirm:$false
    Write-Host "Автозапуск: задача 'MXDS' удалена"
}

# --- осиротевшие службы WinDivert (после штатной остановки их
#     быть не должно; чистим на случай краша) ---
foreach ($svc in @("WinDivert", "WinDivert14", "mxds")) {
    $s = Get-Service -Name $svc -ErrorAction SilentlyContinue
    if ($svc -eq "mxds") { continue }  # служба MXDS не используется (D-008: единый процесс)
    if ($s) {
        & sc.exe stop $svc | Out-Null
        & sc.exe delete $svc | Out-Null
        Write-Host "Служба ${svc}: остановлена и удалена"
    }
}

# --- lock-файл single-instance ---
Remove-Item (Join-Path $InstallDir "mxds.lock") -Force -ErrorAction SilentlyContinue

# --- удаление каталога ---
if (Test-Path $InstallDir) {
    # Уходим из удаляемого каталога: иначе он заблокирован как CWD
    # этого процесса и Remove-Item падает с InvalidOperation.
    Set-Location $env:TEMP
    if ($KeepData) {
        Get-ChildItem $InstallDir -Exclude "config", "logs" | Remove-Item -Recurse -Force
        $keepCfg = Join-Path $InstallDir "config\config.json"
        Get-ChildItem (Join-Path $InstallDir "config") -Exclude "config.json" |
            Remove-Item -Recurse -Force
        Write-Host "Каталог очищен; сохранены: config\config.json, logs\"
    } else {
        Remove-Item $InstallDir -Recurse -Force
        Write-Host "Каталог удалён полностью"
    }
} else {
    Write-Host "Каталог $InstallDir не найден — нечего удалять"
}

Write-Host "== Удаление завершено =="
