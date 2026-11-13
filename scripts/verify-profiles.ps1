#requires -Version 5.1
<#
.SYNOPSIS
    Точечная проверка выбранных профилей с увеличенным тайм-аутом.
.DESCRIPTION
    Запускать ОТ АДМИНИСТРАТОРА. Для каждого профиля из -Profiles:
    движок up → curl целей с тайм-аутом -TimeoutSec → движок down.
    Результаты — verify_profiles.log рядом с дистрибутивом.
#>
[CmdletBinding()]
param(
    [string]$DistDir = $PSScriptRoot,
    [string[]]$Profiles = @("general-alt6", "general-alt7"),
    [string[]]$Targets = @("https://discord.com/", "https://discord.gg/", "https://www.youtube.com/"),
    [int]$TimeoutSec = 30
)

$ErrorActionPreference = "Continue"
$exe = Join-Path $DistDir "mxds.exe"
$log = Join-Path $DistDir "verify_profiles.log"
"=== Точечная проверка $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') ===" |
    Out-File $log -Encoding utf8

foreach ($p in $Profiles) {
    $proc = Start-Process -FilePath $exe -ArgumentList "run --profile $p" `
        -WorkingDirectory $DistDir -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput "$env:TEMP\mxds_vp_out.txt" `
        -RedirectStandardError "$env:TEMP\mxds_vp_err.txt"
    $up = $false
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Seconds 1
        if ((& $exe status 2>&1 | Out-String) -match "Running") { $up = $true; break }
    }
    if (-not $up) {
        "$p : ДВИЖОК НЕ ПОДНЯЛСЯ" | Out-File $log -Append -Encoding utf8
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        continue
    }
    $line = "$p :"
    foreach ($t in $Targets) {
        $r = & curl.exe -s -o NUL -w "%{http_code} %{time_total}" --max-time $TimeoutSec $t 2>&1
        $line += "  $(([uri]$t).Host)=$r"
    }
    $line | Out-File $log -Append -Encoding utf8
    & $exe stop | Out-Null
    Start-Sleep -Seconds 1
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}
"=== Готово $(Get-Date -Format 'HH:mm:ss') ===" | Out-File $log -Append -Encoding utf8
