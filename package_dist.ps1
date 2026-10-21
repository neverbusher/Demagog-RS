[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutputDir = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$buildExe = Join-Path $root "target\release\demagog-rs.exe"
$cacheDir = Join-Path $root "target\release\package"
$glowExe = Join-Path $cacheDir "Demagog-RS.exe"
$wgpuExe = Join-Path $cacheDir "Demagog-RS-wgpu.exe"
$dicSource = Join-Path $root "dic"
$outputRoot = Join-Path $root $OutputDir
$staging = Join-Path $outputRoot "Demagog-RS"

if (-not $Version) {
    $versionLine = Select-String -LiteralPath (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    $Version = if ($versionLine) { $versionLine.Matches[0].Groups[1].Value } else { "0.1.1" }
}
$zipPath = Join-Path $outputRoot "Demagog-RS-$Version-win64.zip"

$running = Get-Process -Name "Demagog-RS", "Demagog-RS-wgpu" -ErrorAction SilentlyContinue
if ($running) {
    throw "Close Demagog-RS and Demagog-RS-wgpu before packaging: the exe files are locked."
}

if (-not $SkipBuild) {
    New-Item -ItemType Directory -Path $cacheDir -Force | Out-Null
    Push-Location $root
    try {
        # Build the DirectX variant first, then leave target/release as the default OpenGL build.
        cargo build --release --features wgpu
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release --features wgpu failed"
        }
        Copy-Item -LiteralPath $buildExe -Destination $wgpuExe -Force

        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed"
        }
        Copy-Item -LiteralPath $buildExe -Destination $glowExe -Force
    }
    finally {
        Pop-Location
    }
}

foreach ($file in @(
    $glowExe,
    $wgpuExe,
    (Join-Path $root "README.md"),
    (Join-Path $root "LICENSE"),
    (Join-Path $root "THIRD_PARTY_NOTICES.md")
)) {
    if (-not (Test-Path -LiteralPath $file)) {
        throw "Missing required file: $file. Run without -SkipBuild first."
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

Copy-Item -LiteralPath $glowExe -Destination (Join-Path $staging "Demagog-RS.exe")
Copy-Item -LiteralPath $wgpuExe -Destination (Join-Path $staging "Demagog-RS-wgpu.exe")
Copy-Item -LiteralPath (Join-Path $root "README.md") -Destination (Join-Path $staging "README.txt")
Copy-Item -LiteralPath (Join-Path $root "LICENSE") -Destination (Join-Path $staging "LICENSE.txt")
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

# Keep the repo-root run artifacts in sync with the packaged binaries.
Copy-Item -LiteralPath $glowExe -Destination (Join-Path $root "Demagog-RS.exe") -Force
Copy-Item -LiteralPath $wgpuExe -Destination (Join-Path $root "Demagog-RS-wgpu.exe") -Force

$zipInfo = Get-Item -LiteralPath $zipPath
$glowInfo = Get-Item -LiteralPath $glowExe
$wgpuInfo = Get-Item -LiteralPath $wgpuExe
Write-Output ("Built: {0}" -f $zipPath)
Write-Output ("Exe OpenGL: {0:N1} MB" -f ($glowInfo.Length / 1MB))
Write-Output ("Exe wgpu:   {0:N1} MB" -f ($wgpuInfo.Length / 1MB))
Write-Output ("Zip:        {0:N1} MB" -f ($zipInfo.Length / 1MB))
Write-Output ("Dictionaries: {0}" -f $dictionaries.Count)
Write-Output ("SHA256: {0}" -f $hash)
