# Downloads libmpv (shinchiro Windows build) and prepares it for linking.
# Idempotent: re-runs safely. Run once after cloning, or to update libmpv.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File .\scripts\setup-libmpv.ps1
#   powershell -ExecutionPolicy Bypass -File .\scripts\setup-libmpv.ps1 -Force

[CmdletBinding()]
param(
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$VendorDir  = Join-Path $RepoRoot 'src-tauri\vendor\libmpv'
$ArchiveDir = Join-Path $VendorDir '_archive'
New-Item -ItemType Directory -Force -Path $VendorDir, $ArchiveDir | Out-Null

$Dll  = Join-Path $VendorDir 'libmpv-2.dll'
$Lib  = Join-Path $VendorDir 'mpv.lib'
$Hdr  = Join-Path $VendorDir 'include\mpv\client.h'
if (-not $Force -and (Test-Path $Dll) -and (Test-Path $Lib) -and (Test-Path $Hdr)) {
    Write-Host "libmpv already set up at $VendorDir. Use -Force to refresh." -ForegroundColor Green
    exit 0
}

Write-Host 'Querying latest shinchiro/mpv-winbuild-cmake release...'
$headers = @{ 'User-Agent' = 'PlayerSage-setup' }
$release = Invoke-RestMethod -Headers $headers -Uri 'https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest'

$asset = $release.assets | Where-Object { $_.name -like 'mpv-dev-x86_64-v3-*.7z' } | Select-Object -First 1
if (-not $asset) {
    $asset = $release.assets | Where-Object { $_.name -like 'mpv-dev-x86_64-*.7z' } | Select-Object -First 1
}
if (-not $asset) {
    throw "No mpv-dev-x86_64 asset found in release $($release.tag_name)."
}

$ArchivePath = Join-Path $ArchiveDir $asset.name
if ($Force -or -not (Test-Path $ArchivePath)) {
    $sizeMB = [Math]::Round($asset.size / 1MB, 1)
    Write-Host "Downloading $($asset.name) ($sizeMB MB)..."
    Invoke-WebRequest -Headers $headers -Uri $asset.browser_download_url -OutFile $ArchivePath
} else {
    Write-Host "Using cached archive $ArchivePath"
}

$SevenZipCandidates = @(
    'C:\Program Files\7-Zip\7z.exe',
    'C:\Program Files (x86)\7-Zip\7z.exe'
)
$SevenZip = $SevenZipCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $SevenZip) {
    $SevenZip = (Get-Command 7z.exe -ErrorAction SilentlyContinue).Source
}
if (-not $SevenZip) {
    throw "7-Zip not found. Install it from https://7-zip.org/ (need 7z.exe to extract .7z)."
}

Get-ChildItem -Path $VendorDir -Exclude '_archive' -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
Write-Host 'Extracting archive...'
& $SevenZip x -y "-o$VendorDir" $ArchivePath | Out-Null
if (-not (Test-Path $Dll)) {
    throw "Extraction succeeded but $Dll is missing. Archive layout may have changed."
}

$Def = Join-Path $VendorDir 'mpv.def'

# Locate MSVC binaries (dumpbin, lib) directly by finding the latest
# VC\Tools\MSVC\<version>\bin\Hostx64\x64 directory. Avoids cmd /c quoting issues.
function Get-MsvcBinDir {
    $onPathLib = (Get-Command lib.exe -ErrorAction SilentlyContinue).Source
    if ($onPathLib) { return (Split-Path -Parent $onPathLib) }

    $vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) {
        throw "vswhere.exe not found and lib.exe not on PATH. Install VS 2022 Build Tools (C++ workload)."
    }
    $vsRoot = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vsRoot) {
        throw "No VS installation with VC++ tools found."
    }
    $msvcRoot = Join-Path $vsRoot 'VC\Tools\MSVC'
    if (-not (Test-Path $msvcRoot)) {
        throw "MSVC tools directory not found at $msvcRoot."
    }
    $latest = Get-ChildItem $msvcRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1
    if (-not $latest) {
        throw "No MSVC version directories under $msvcRoot."
    }
    $binDir = Join-Path $latest.FullName 'bin\Hostx64\x64'
    if (-not (Test-Path (Join-Path $binDir 'lib.exe'))) {
        throw "lib.exe not found in $binDir."
    }
    return $binDir
}

$MsvcBin = Get-MsvcBinDir
$LibExe  = Join-Path $MsvcBin 'lib.exe'
$Dumpbin = Join-Path $MsvcBin 'dumpbin.exe'
Write-Host "Using MSVC tools at $MsvcBin"

if (-not (Test-Path $Def)) {
    Write-Host 'Generating mpv.def from libmpv-2.dll exports...'
    $dumpOutput = & $Dumpbin /exports $Dll
    if ($LASTEXITCODE -ne 0) {
        throw "dumpbin failed (exit $LASTEXITCODE)."
    }

    $exports = New-Object System.Collections.Generic.List[string]
    $inExports = $false
    foreach ($line in $dumpOutput) {
        $text = $line.ToString()
        if ($text -match '^\s*ordinal\s+hint\s+RVA\s+name\s*$') {
            $inExports = $true
            continue
        }
        if (-not $inExports) { continue }
        if ($text -match '^\s*Summary\s*$') { break }
        if ($text -match '^\s*\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\S+)\s*$') {
            $exports.Add($Matches[1])
        }
    }
    if ($exports.Count -eq 0) {
        throw "dumpbin returned no exports."
    }

    $defLines = @('LIBRARY libmpv-2', 'EXPORTS') + $exports
    Set-Content -Path $Def -Value $defLines -Encoding ASCII
    Write-Host "  wrote $($exports.Count) exports to $Def"
}

Write-Host 'Running lib.exe to produce mpv.lib...'
& $LibExe /nologo "/def:$Def" "/out:$Lib" /machine:x64 | Out-Null
if ($LASTEXITCODE -ne 0) { throw "lib.exe failed (exit $LASTEXITCODE)." }

if (-not (Test-Path $Lib)) {
    throw "Failed to generate $Lib."
}

Write-Host "libmpv installed at $VendorDir" -ForegroundColor Green
Write-Host "  DLL: $Dll"
Write-Host "  LIB: $Lib"
