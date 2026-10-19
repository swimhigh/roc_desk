$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-workspace-migrate'
if (Test-Path $d) { Remove-Item -Recurse -Force $d }
git clone https://github.com/swimhigh/roc_desk-workspace.git $d
New-Item -ItemType Directory -Force "$d/lib/src/coding", "$d/lib/src/pty", "$d/lib/src/symbols", "$d/src-web/src/components/CodingAgent", "$d/src-web/src/stores", "$d/src-web/src/services" | Out-Null
Copy-Item -Recurse 'src-tauri/src/coding/*' "$d/lib/src/coding/"
Copy-Item -Recurse 'src-tauri/src/pty/*' "$d/lib/src/pty/"
Copy-Item -Recurse 'src-tauri/src/symbols/*' "$d/lib/src/symbols/"
Copy-Item 'src-web/src/components/CodingAgent/*' "$d/src-web/src/components/CodingAgent/"
Copy-Item 'src-web/src/stores/codingStore.ts' "$d/src-web/src/stores/"
Copy-Item 'src-web/src/services/codingService.ts' "$d/src-web/src/services/"
@'
# roc_desk-workspace migration

Coding workspace, local PTY, symbols, AI coding panel, store, and service
sources are now tracked here. This tool will consume `roc_desk-editor` as a
one-way dependency and `roc_desk-common` for shared AI/core interfaces.

The host copy remains until those dependencies and the standalone shell are
compiled independently.
'@ | Set-Content "$d/MIGRATION.md" -Encoding UTF8
Push-Location $d; git config user.name 'lipeng'; git config user.email 'lipeng@szkingdom.com'; git add .; git commit -m 'Migrate coding workspace source' | Out-Null; git push origin main; Pop-Location
