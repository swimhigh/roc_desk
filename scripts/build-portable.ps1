$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseDir = Join-Path $repoRoot 'build\release'
$portableDir = Join-Path $repoRoot 'bin'
$exePath = Join-Path $releaseDir 'roc_desk.exe'

Write-Host '[roc_desk] Building production bundle...' -ForegroundColor Cyan
Push-Location $repoRoot
try {
    # tauri:build also runs beforeBuildCommand from tauri.conf.json (build:web).
    npm run tauri:build
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "Release executable not found: $exePath"
}

# Resolve and validate the exact target before recursively cleaning it.
$resolvedRoot = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd('\')
$resolvedBin = [System.IO.Path]::GetFullPath($portableDir).TrimEnd('\')
if (-not $resolvedBin.StartsWith($resolvedRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to clean output outside repository: $resolvedBin"
}
if (-not (Test-Path -LiteralPath $portableDir)) {
    New-Item -ItemType Directory -Path $portableDir -Force | Out-Null
} else {
    # Explorer, antivirus software, or the current EXE can temporarily lock the
    # directory itself. Keep the directory and clean its children so the copy
    # step is not aborted merely because the bin directory handle is open.
    Get-ChildItem -LiteralPath $portableDir -Force |
        Where-Object { $_.Name -ne '.rock_desk' } |
        ForEach-Object {
        try {
            Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction Stop
        } catch {
            Write-Warning "Could not remove $($_.FullName): $($_.Exception.Message)"
        }
        }
}

Copy-Item -LiteralPath $exePath -Destination (Join-Path $portableDir 'roc_desk.exe') -Force
# Copy side-by-side runtime DLLs if future Tauri settings produce any.
Get-ChildItem -LiteralPath $releaseDir -File -Filter '*.dll' -ErrorAction SilentlyContinue |
    Copy-Item -Destination $portableDir -Force

$readmeTemplate = Join-Path $PSScriptRoot 'README-portable.txt'
if (-not (Test-Path -LiteralPath $readmeTemplate -PathType Leaf)) {
    throw "README template not found: $readmeTemplate"
}
Copy-Item -LiteralPath $readmeTemplate -Destination (Join-Path $portableDir 'README.txt') -Force

Write-Host "[roc_desk] Portable files written to $portableDir" -ForegroundColor Green
Get-ChildItem -LiteralPath $portableDir | Select-Object Name, Length
