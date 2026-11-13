#requires -Version 5.1
<#
.SYNOPSIS
    Установка MXDS из распакованного дистрибутива.
.DESCRIPTION
    Цикл 16 (Release). Проверяет зависимости (Windows 10/11 x64),
    копирует полезную нагрузку в каталог установки, проверяет
    целостность по SHA256SUMS.txt, опционально регистрирует
    автозапуск движка через Планировщик задач (при входе, с
    наивысшими правами) — UX «запустил и забыл» (D-003).
.EXAMPLE
    powershell -File install.ps1
    powershell -File install.ps1 -InstallDir D:\Tools\MXDS -Autostart
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\MXDS"),
    [switch]$Autostart
)

$ErrorActionPreference = "Stop"
$payload = $PSScriptRoot   # скрипт лежит в корне распакованного дистрибутива

# --- проверка зависимостей ---
if (-not [Environment]::Is64BitOperatingSystem) { throw "требуется 64-разрядная Windows" }
$build = [int](Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion").CurrentBuildNumber
if ($build -lt 17763) { throw "требуется Windows 10 1809+ (build >= 17763), обнаружен build $build" }
if (-not (Test-Path (Join-Path $payload "mxds.exe"))) {
    throw "mxds.exe не найден рядом со скриптом — запускайте install.ps1 из распакованного дистрибутива"
}
# CRT встроен статически (x86_64-pc-windows-msvc), внешних
# зависимостей нет; WinDivert поставляется в assets\.

Write-Host "== Установка MXDS в $InstallDir =="

# --- копирование ---
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Get-ChildItem $payload | ForEach-Object {
    Copy-Item $_.FullName $InstallDir -Recurse -Force
}

# --- проверка целостности после копирования ---
$verify = Join-Path $InstallDir "verify-install.ps1"
& powershell -NoProfile -ExecutionPolicy Bypass -File $verify -InstallDir $InstallDir -Quiet
if ($LASTEXITCODE -ne 0) { throw "проверка целостности не пройдена — установка прервана" }
Write-Host "Целостность файлов: OK"

# --- автозапуск (опционально) ---
if ($Autostart) {
    try {
        $exe = Join-Path $InstallDir "mxds.exe"
        $action = New-ScheduledTaskAction -Execute $exe -Argument "run"
        $trigger = New-ScheduledTaskTrigger -AtLogOn
        $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -RunLevel Highest
        Register-ScheduledTask -TaskName "MXDS" -Action $action -Trigger $trigger `
            -Principal $principal -Force -Description "MXDS — обход DPI (автозапуск движка)" | Out-Null
        Write-Host "Автозапуск: задача 'MXDS' зарегистрирована в Планировщике (при входе, наивысшие права)"
    } catch {
        Write-Warning "Автозапуск не настроен: $($_.Exception.Message)"
        Write-Warning "Запускайте вручную: mxds.exe run (от администратора)"
    }
}

Write-Host ""
Write-Host "== Установлено =="
Write-Host "  Каталог: $InstallDir"
Write-Host "  Запуск GUI:   $InstallDir\mxds.exe"
Write-Host "  Запуск CLI:   $InstallDir\mxds.exe run    (от администратора)"
Write-Host "  Статус:       $InstallDir\mxds.exe status"
Write-Host "  Удаление:     powershell -File $InstallDir\uninstall.ps1"
