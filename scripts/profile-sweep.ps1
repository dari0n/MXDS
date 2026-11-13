#requires -Version 5.1
<#
.SYNOPSIS
    Перебор профилей MXDS: для каждого профиля поднимает движок,
    проверяет отклик целей (discord.com, youtube.com), останавливает.
.DESCRIPTION
    Запускать ОТ АДМИНИСТРАТОРА (Start-Process -Verb RunAs).
    Один UAC на весь перебор. Результаты — в profile_sweep.log рядом.
.EXAMPLE
    powershell -File profile-sweep.ps1 -DistDir C:\path\to\MXDS-0.1.0-win-x64
#>
[CmdletBinding()]
param(
    [string]$DistDir = $PSScriptRoot,
    [string[]]$Targets = @("https://discord.com/", "https://www.youtube.com/"),
    [int]$TimeoutSec = 12
)

$ErrorActionPreference = "Continue"
$exe = Join-Path $DistDir "mxds.exe"
$log = Join-Path $DistDir "profile_sweep.log"
if (-not (Test-Path $exe)) { throw "mxds.exe не найден в $DistDir" }

"=== Перебор профилей $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') ===" |
    Out-File $log -Encoding utf8

$profiles = Get-ChildItem (Join-Path $DistDir "profiles\*.json") |
    ForEach-Object { $_.BaseName } | Sort-Object
"профилей: $($profiles.Count)" | Out-File $log -Append -Encoding utf8

foreach ($p in $profiles) {
    # Запуск движка с профилем (фоном, вывод в nul).
    $proc = Start-Process -FilePath $exe -ArgumentList "run --profile $p" `
        -WorkingDirectory $DistDir -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput "$env:TEMP\mxds_sweep_out.txt" `
        -RedirectStandardError "$env:TEMP\mxds_sweep_err.txt"

    # Ждём поднятия движка (poll status до 10 с). NB: русский текст
    # из stdout может прийти в чужой кодовой странице — матчим ASCII.
    $up = $false
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Seconds 1
        $st = & $exe status 2>&1 | Out-String
        if ($st -match "Running") { $up = $true; break }
    }
    if (-not $up) {
        "$p : ДВИЖОК НЕ ПОДНЯЛСЯ" | Out-File $log -Append -Encoding utf8
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        continue
    }

    $line = "$p :"
    foreach ($t in $Targets) {
        $r = & curl.exe -s -o NUL -w "%{http_code} %{time_total}" `
            --max-time $TimeoutSec $t 2>&1
        $host_ = ([uri]$t).Host
        $line += "  $host_=$r"
    }
    $line | Out-File $log -Append -Encoding utf8

    & $exe stop | Out-Null
    Start-Sleep -Seconds 1
    # Статистика движка за прогон (из stdout дочернего процесса).
    $engineOut = Get-Content "$env:TEMP\mxds_sweep_err.txt" -Raw -ErrorAction SilentlyContinue
    if ($engineOut -match "received=(\d+)\s+sent=(\d+)\s+dropped=(\d+)") {
        "  stats: recv=$($Matches[1]) sent=$($Matches[2]) dropped=$($Matches[3])" |
            Out-File $log -Append -Encoding utf8
    }
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

"=== Готово $(Get-Date -Format 'HH:mm:ss') ===" | Out-File $log -Append -Encoding utf8
