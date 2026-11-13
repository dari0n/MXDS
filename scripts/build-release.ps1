#requires -Version 5.1
<#
.SYNOPSIS
    MXDS release build: компиляция, стейджинг дистрибутива, ZIP, контрольные суммы.
.DESCRIPTION
    Цикл 16 (Release). Собирает cargo --release, формирует
    dist/MXDS-<version>-win-x64/ с полезной нагрузкой, генерирует
    SHA256SUMS.txt (проверка целостности) и ZIP-архив.
.EXAMPLE
    powershell -File scripts\build-release.ps1
#>
[CmdletBinding()]
param(
    [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# --- cargo ---
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $candidate) { $cargo = $candidate } else { throw "cargo не найден в PATH" }
}

# --- версия из Cargo.toml ---
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches.Groups[1].Value
if (-not $version) { throw "версия не найдена в Cargo.toml" }
Write-Host "== MXDS ${version}: сборка ($Configuration) =="

# --- сборка ---
& $cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build --release завершился кодом $LASTEXITCODE" }

# --- стейджинг ---
$stageName = "MXDS-$version-win-x64"
$dist = Join-Path $root "dist"
$stage = Join-Path $dist $stageName
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage | Out-Null

$exe = Join-Path $root "target\release\mxds.exe"
if (-not (Test-Path $exe)) { throw "не найден $exe" }

Copy-Item $exe $stage
foreach ($dir in @("assets", "profiles", "lists", "config")) {
    Copy-Item (Join-Path $root $dir) (Join-Path $stage $dir) -Recurse
}
# Пользовательская конфигурация разработчика в дистрибутив не входит.
Remove-Item (Join-Path $stage "config\config.json") -Force -ErrorAction SilentlyContinue
Copy-Item (Join-Path $root "README.md") $stage
Copy-Item (Join-Path $root "CHANGELOG.md") $stage
Copy-Item (Join-Path $root "docs") (Join-Path $stage "docs") -Recurse
foreach ($s in @("install.ps1", "uninstall.ps1", "verify-install.ps1")) {
    # Принудительно UTF-8 с BOM: PowerShell 5.1 (дефолт в Windows)
    # иначе читает кириллицу как ANSI и ломает парсинг.
    $text = Get-Content (Join-Path $root "scripts\$s") -Raw -Encoding UTF8
    [System.IO.File]::WriteAllText(
        (Join-Path $stage $s), $text, [System.Text.UTF8Encoding]::new($true))
}

# --- контрольные суммы (целостность) ---
$sums = Get-ChildItem $stage -Recurse -File |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    Sort-Object FullName |
    ForEach-Object {
        $rel = $_.FullName.Substring($stage.Length + 1) -replace '\\', '/'
        "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $rel
    }
Set-Content -Path (Join-Path $stage "SHA256SUMS.txt") -Value $sums -Encoding utf8

# --- ZIP ---
$zip = Join-Path $dist "$stageName.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
$zipHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
Set-Content -Path "$zip.sha256" -Value "$zipHash  $(Split-Path $zip -Leaf)" -Encoding ascii

$exeSize = [math]::Round((Get-Item $exe).Length / 1MB, 2)
$zipSize = [math]::Round((Get-Item $zip).Length / 1MB, 2)
Write-Host ""
Write-Host "== Готово =="
Write-Host "  mxds.exe : $exeSize МиБ"
Write-Host "  Дистрибутив: $stage"
Write-Host "  ZIP ($zipSize МиБ): $zip"
Write-Host "  SHA256 ZIP: $zipHash"
