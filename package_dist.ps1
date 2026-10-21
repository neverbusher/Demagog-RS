[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutputDir = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exeSource = Join-Path $root "target\release\demagog-rs.exe"
$dicSource = Join-Path $root "dic"
$outputRoot = Join-Path $root $OutputDir
$staging = Join-Path $outputRoot "Demagog-RS"

if (-not $Version) {
    $versionLine = Select-String -LiteralPath (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    $Version = if ($versionLine) { $versionLine.Matches[0].Groups[1].Value } else { "0.1.0" }
}
$zipPath = Join-Path $outputRoot "Demagog-RS-$Version-win64.zip"

if (-not $SkipBuild) {
    if (Get-Process -Name "Demagog-RS" -ErrorAction SilentlyContinue) {
        throw "Close Demagog-RS before packaging: target\release\demagog-rs.exe is locked."
    }
    Push-Location $root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed"
        }
    }
    finally {
        Pop-Location
    }
}

foreach ($file in @(
    $exeSource,
    (Join-Path $root "README.md"),
    (Join-Path $root "THIRD_PARTY_NOTICES.md")
)) {
    if (-not (Test-Path -LiteralPath $file)) {
        throw "Missing required file: $file"
    }
}
if (-not (Test-Path -LiteralPath $dicSource -PathType Container)) {
    throw "Missing dic folder: $dicSource"
}

# Only the staging directory owned by this script is removed.
if (Test-Path -LiteralPath $staging) {
    Remove-Item -LiteralPath $staging -Recurse -Force
}
New-Item -ItemType Directory -Path $staging | Out-Null
New-Item -ItemType Directory -Path (Join-Path $staging "dic") | Out-Null

Copy-Item -LiteralPath $exeSource -Destination (Join-Path $staging "Demagog-RS.exe")
Copy-Item -LiteralPath (Join-Path $root "README.md") -Destination (Join-Path $staging "README.txt")
Copy-Item -LiteralPath (Join-Path $root "THIRD_PARTY_NOTICES.md") -Destination (Join-Path $staging "THIRD_PARTY_NOTICES.md")

$excludedDictionaries = @("35_мой словарь.dic")
$dictionaries = @(Get-ChildItem -LiteralPath $dicSource -File | Where-Object {
    ($_.Extension -eq ".rex" -or $_.Extension -eq ".dic") -and
    ($excludedDictionaries -notcontains $_.Name)
})
if ($dictionaries.Count -eq 0) {
    throw "No dictionaries found in $dicSource"
}
$dictionaries | Copy-Item -Destination (Join-Path $staging "dic") -Force

if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}
Compress-Archive -Path $staging -DestinationPath $zipPath -CompressionLevel Optimal

$hash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
$hashPath = "$zipPath.sha256"
Set-Content -LiteralPath $hashPath -Value ("{0}  {1}" -f $hash, (Split-Path -Leaf $zipPath)) -Encoding ASCII

$zipInfo = Get-Item -LiteralPath $zipPath
$exeInfo = Get-Item -LiteralPath $exeSource
Write-Output ("Built: {0}" -f $zipPath)
Write-Output ("Exe:   {0:N1} MB" -f ($exeInfo.Length / 1MB))
Write-Output ("Zip:   {0:N1} MB" -f ($zipInfo.Length / 1MB))
Write-Output ("Dictionaries: {0}" -f $dictionaries.Count)
Write-Output ("SHA256: {0}" -f $hash)
