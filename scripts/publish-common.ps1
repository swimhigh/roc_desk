$ErrorActionPreference = 'Stop'
$publishDir = Join-Path $env:TEMP 'roc_desk-common-publish'
if (Test-Path $publishDir) { Remove-Item -Recurse -Force $publishDir }
New-Item -ItemType Directory -Path $publishDir | Out-Null
Copy-Item -Recurse 'src-tauri/core' (Join-Path $publishDir 'core')
Copy-Item -Recurse 'src-tauri/common' (Join-Path $publishDir 'common')
New-Item -ItemType Directory -Path (Join-Path $publishDir 'packages/ui-core/src') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $publishDir 'packages/common-web/src') -Force | Out-Null
Copy-Item 'packages/ui-core/package.json' (Join-Path $publishDir 'packages/ui-core/package.json')
Copy-Item 'packages/ui-core/src/index.ts' (Join-Path $publishDir 'packages/ui-core/src/index.ts')
Copy-Item 'src-web/src/types/bindings.ts' (Join-Path $publishDir 'packages/ui-core/src/bindings.ts')
Copy-Item 'packages/common-web/package.json' (Join-Path $publishDir 'packages/common-web/package.json')
@'
/** Optional browser/log UI shared by tools. */
export type { SearchMode, SearchOptions, SearchMatch } from "@roc_desk/ui-core";
'@ | Set-Content (Join-Path $publishDir 'packages/common-web/src/index.ts') -Encoding UTF8
@'
[workspace]
resolver = "2"
members = ["core", "common"]
'@ | Set-Content (Join-Path $publishDir 'Cargo.toml') -Encoding UTF8
@'
# roc_desk-common

Shared Rust and frontend packages for the roc_desk tool repositories.

This repository contains libraries only. It has no Tauri executable and never
imports the roc_desk host or any tool repository.
'@ | Set-Content (Join-Path $publishDir 'README.md') -Encoding UTF8
@'
node_modules/
target/
Cargo.lock
'@ | Set-Content (Join-Path $publishDir '.gitignore') -Encoding UTF8
Push-Location $publishDir
git init -b main | Out-Null
git config user.name 'lipeng'
git config user.email 'lipeng@szkingdom.com'
git add .
git commit -m 'Initialize shared core and frontend packages' | Out-Null
git remote add origin https://github.com/swimhigh/roc_desk-common.git
git push -u origin main
Pop-Location
Write-Output "PUBLISHED $publishDir"
