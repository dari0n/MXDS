#requires -Version 5.1
# Точечная проверка эндпоинтов Discord: general vs general-alt6.
# Запускать ОТ АДМИНИСТРАТОРА. Результаты — discord_check.log рядом.
$ErrorActionPreference = "Continue"
$DistDir = "C:\Users\MINX\Documents\rust\MXDS\dist\MXDS-0.1.0-win-x64"
$exe = Join-Path $DistDir "mxds.exe"
$log = Join-Path $DistDir "discord_check.log"
$targets = @("https://gateway.discord.gg/", "https://cdn.discordapp.com/", "https://discord.com/", "https://www.youtube.com/")

"=== Discord endpoints check $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') ===" |
    Out-File $log -Encoding utf8

foreach ($p in @("general", "general-alt6")) {
    $proc = Start-Process -FilePath $exe -ArgumentList "run --profile $p" `
        -WorkingDirectory $DistDir -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput "$env:TEMP\mxds_dc_out.txt" `
        -RedirectStandardError "$env:TEMP\mxds_dc_err.txt"
    $up = $false
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Seconds 1
        if ((& $exe status 2>&1 | Out-String) -match "Running") { $up = $true; break }
    }
    if (-not $up) {
        "$p : ENGINE DID NOT START" | Out-File $log -Append -Encoding utf8
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        continue
    }
    foreach ($t in $targets) {
        $r = & curl.exe -s -o NUL -w "%{http_code} %{time_total}" --max-time 25 $t 2>&1
        "$p : $(([uri]$t).Host) = $r" | Out-File $log -Append -Encoding utf8
    }
    & $exe stop | Out-Null
    Start-Sleep -Seconds 1
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}
"=== Done $(Get-Date -Format 'HH:mm:ss') ===" | Out-File $log -Append -Encoding utf8
