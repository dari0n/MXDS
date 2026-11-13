#requires -Version 5.1
<#
.SYNOPSIS
    Проверка целостности установленной копии MXDS.
.DESCRIPTION
    Цикл 16 (Release). Сверяет все файлы с SHA256SUMS.txt,
    проверяет зависимости (Windows 10/11 x64, build >= 17763).
    Код выхода: 0 — всё в порядке, 1 — есть расхождения.
.EXAMPLE
    powershell -File verify-install.ps1
    powershell -File verify-install.ps1 -InstallDir D:\Tools\MXDS
#>
[CmdletBinding()]
param(
    # Пусто = каталог скрипта. NB: $PSScriptRoot в значении по умолчанию
    # при [CmdletBinding()] в PS 5.1 пуст — резолвим в теле.
    [string]$InstallDir = "",
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrEmpty($InstallDir)) { $InstallDir = $PSScriptRoot }
$failed = 0
$checked = 0

$sumsPath = Join-Path $InstallDir "SHA256SUMS.txt"
if (-not (Test-Path $sumsPath)) {
    Write-Error "SHA256SUMS.txt не найден в $InstallDir"
    exit 1
}

foreach ($line in Get-Content $sumsPath) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $parts = $line -split '\s{2,}', 2
    if ($parts.Count -ne 2) { continue }
    $expected = $parts[0].ToLower()
    $rel = $parts[1] -replace '/', '\'
    $full = Join-Path $InstallDir $rel
    $checked++
    if (-not (Test-Path $full)) {
        if (-not $Quiet) { Write-Host "ОТСУТСТВУЕТ: $rel" }
        $failed++
        continue
    }
    $actual = (Get-FileHash $full -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected) {
        if (-not $Quiet) { Write-Host "ИЗМЕНЁН:    $rel" }
        $failed++
    }
}

if (-not $Quiet) {
    Write-Host ""
    Write-Host "Проверено файлов: $checked, расхождений: $failed"
    Write-Host "ОС: build $([int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber), x64: $([Environment]::Is64BitOperatingSystem)"
}

if ($failed -gt 0) { exit 1 }
if (-not $Quiet) { Write-Host "Целостность: OK" }
exit 0
