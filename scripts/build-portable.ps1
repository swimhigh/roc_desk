$ErrorActionPreference = 'Stop'

# 便携版正在运行时会锁住 bin\roc_desk.exe——之前踩过一次：目录已经清空但拷贝
# 因为文件被占用而失败，脚本中止，留下一个只剩 .rock_desk 的半损坏 bin\ 目录。
# 构建前先关掉任何还在跑的实例，避免重现这个问题。
Get-Process roc_desk -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseDir = Join-Path $repoRoot 'build\release'
$portableDir = Join-Path $repoRoot 'bin'
$exePath = Join-Path $releaseDir 'roc_desk.exe'
# roc_desk_agent.exe 故意编译到 `x86_64-win7-windows-msvc`（Tier 3 目标），
# 不是默认的 `x86_64-pc-windows-msvc`——从 Rust 1.78 起，默认目标产出的二进制
# 静态导入表里带了 `api-ms-win-core-synch-l1-2-0.dll`（新同步原语 WaitOnAddress，
# 这个 apiset 重定向 DLL 是 Windows 8.1 / Server 2012 R2 才引入的 API-Set Schema v2
# 带来的），在没有这个 apiset 的老系统（Windows 7、Server 2008 R2、**Server 2012**
# 非 R2）上会直接加载失败——而这些老 Windows Server 恰恰是 AGENT_DESIGN.md §一
# 开篇点名的主要目标场景（"没装/不方便装 OpenSSH 的老版本 Server"）。win7 目标
# 编译出的 std 对这类新 API 一律先用 GetProcAddress 探测、探测不到就退回老实现，
# 静态导入表里完全不出现这个 apiset，同一份二进制向下兼容到 Windows 7 SP1，
# 在新版本 Windows 上运行完全没有副作用（已用 dumpbin /imports 核对过两种目标的
# 导入表差异，并跑过完整的 TLS 握手 + 协议往返自测，见 agent/src/selftest.rs）。
$agentTarget = 'x86_64-win7-windows-msvc'
$agentExePath = Join-Path $repoRoot "build\$agentTarget\release\roc_desk_agent.exe"

Write-Host '[roc_desk] Building production bundle...' -ForegroundColor Cyan
Push-Location $repoRoot
try {
    # tauri:build also runs beforeBuildCommand from tauri.conf.json (build:web).
    npm run tauri:build
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed with exit code $LASTEXITCODE" }

    # roc_desk_agent.exe（AGENT_DESIGN.md 的远程 Windows Agent）是独立的部署产物，
    # 部署在被管理的远程服务器上，不是 tauri bundle 的一部分——tauri:build 只编译
    # roc_desk 这一个 crate，这里单独编译 agent crate 一起放进便携版目录，方便
    # 用户从同一个下载包里拿到两者。客户端 roc_desk.exe 继续用默认 stable 工具链/
    # 目标（它跑在用户自己的现代机器上，代码里也用到了比较新的标准库方法，
    # 不适合、也不需要降级）；只有 agent 这一个产物需要照顾老系统兼容性。
    #
    # win7 目标是 Tier 3——没有预编译的标准库，需要 nightly 工具链现场用
    # `-Z build-std` 从源码编译一份；这几步是一次性的环境准备，已装好时秒过。
    Write-Host '[roc_desk] Preparing nightly toolchain for roc_desk_agent (win7-compatible target)...' -ForegroundColor Cyan
    rustup toolchain install nightly --profile minimal | Out-Null
    rustup component add rust-src --toolchain nightly | Out-Null
    rustup component add cargo --toolchain nightly | Out-Null

    Write-Host "[roc_desk] Building roc_desk_agent for $agentTarget..." -ForegroundColor Cyan
    cargo +nightly build -p roc_desk_agent --release --target $agentTarget -Z build-std=std,panic_unwind
    if ($LASTEXITCODE -ne 0) { throw "cargo build -p roc_desk_agent (target=$agentTarget) failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "Release executable not found: $exePath"
}
if (-not (Test-Path -LiteralPath $agentExePath -PathType Leaf)) {
    throw "Release executable not found: $agentExePath"
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

# roc_desk_agent.exe：这个文件是要拷到*远程被管理的 Windows 服务器*上运行的，
# 不是给运行 roc_desk.exe 的这台机器用的——单独放进 bin\agent\ 子目录（而不是和
# 客户端文件混在一起），一眼就能看出"这一份要单独拷到别的机器上"，也方便整个
# 子目录直接复制走。
$agentDir = Join-Path $portableDir 'agent'
New-Item -ItemType Directory -Path $agentDir -Force | Out-Null
Copy-Item -LiteralPath $agentExePath -Destination (Join-Path $agentDir 'roc_desk_agent.exe') -Force
$agentReadme = Join-Path $PSScriptRoot 'README-agent.txt'
if (-not (Test-Path -LiteralPath $agentReadme -PathType Leaf)) {
    throw "README template not found: $agentReadme"
}
Copy-Item -LiteralPath $agentReadme -Destination (Join-Path $agentDir 'README.txt') -Force

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
