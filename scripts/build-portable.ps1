$ErrorActionPreference = 'Stop'

# 便携版正在运行时会锁住 bin\roc_desk.exe——之前踩过一次：目录已经清空但拷贝
# 因为文件被占用而失败，脚本中止，留下一个只剩 .rock_desk 的半损坏 bin\ 目录。
# 构建前先关掉任何还在跑的实例，避免重现这个问题。
Get-Process roc_desk -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

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

# RDP（远程工具模式）内嵌用的 FreeRDP 客户端——vendor/ 下是校验过 SHA256 的官方 CI
# 产物（见 src-tauri/src/rdp/mod.rs 顶部注释），运行时从 roc_desk.exe 同目录查找，
# 便携版必须把它一起放进 bin/。
$vendorExe = Join-Path $repoRoot 'vendor\wfreerdp.exe'
if (-not (Test-Path -LiteralPath $vendorExe -PathType Leaf)) {
    throw "vendor\wfreerdp.exe not found — RDP embedding depends on it"
}
Copy-Item -LiteralPath $vendorExe -Destination (Join-Path $portableDir 'wfreerdp.exe') -Force
$vendorLicense = Join-Path $repoRoot 'vendor\wfreerdp.LICENSE.txt'
if (Test-Path -LiteralPath $vendorLicense -PathType Leaf) {
    Copy-Item -LiteralPath $vendorLicense -Destination (Join-Path $portableDir 'wfreerdp.LICENSE.txt') -Force
}

$readmeTemplate = Join-Path $PSScriptRoot 'README-portable.txt'
if (-not (Test-Path -LiteralPath $readmeTemplate -PathType Leaf)) {
    throw "README template not found: $readmeTemplate"
}
Copy-Item -LiteralPath $readmeTemplate -Destination (Join-Path $portableDir 'README.txt') -Force

Write-Host "[roc_desk] Portable files written to $portableDir" -ForegroundColor Green
Get-ChildItem -LiteralPath $portableDir | Select-Object Name, Length
