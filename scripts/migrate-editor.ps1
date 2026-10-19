$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-editor-migrate'
if (Test-Path $d) { Remove-Item -Recurse -Force $d }
git clone https://github.com/swimhigh/roc_desk-editor.git $d
New-Item -ItemType Directory -Force "$d/src-web/src/components/Editor", "$d/src-web/src/stores", "$d/src-web/src/services", "$d/src-web/src/hooks", "$d/src-web/src/utils" | Out-Null
Copy-Item 'src-web/src/components/Editor/*' "$d/src-web/src/components/Editor/"
Copy-Item 'src-web/src/stores/editorStore.ts' "$d/src-web/src/stores/"
Copy-Item 'src-web/src/services/localFsService.ts' "$d/src-web/src/services/"
Copy-Item 'src-web/src/services/fsService.ts' "$d/src-web/src/services/"
Copy-Item 'src-web/src/hooks/useExternalFileDrop.ts' "$d/src-web/src/hooks/"
Copy-Item 'src-web/src/utils/*' "$d/src-web/src/utils/"
@'
# roc_desk-editor migration

The editor components, editor store, file adapters, and standalone file-drop
logic are now tracked here as the canonical migration source. The host keeps a
compatibility copy while the filesystem/symbol interfaces are extracted into
the common core and the standalone Tauri/web shell is added.

The workspace tool will eventually consume the editor package as a one-way
dependency; editor must not depend on workspace.
'@ | Set-Content "$d/MIGRATION.md" -Encoding UTF8
Push-Location $d
git config user.name 'lipeng'
git config user.email 'lipeng@szkingdom.com'
git add .
git commit -m 'Migrate editor components and standalone file support' | Out-Null
git push origin main
Pop-Location
Write-Output $d
