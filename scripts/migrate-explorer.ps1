$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-explorer-migrate'
if (Test-Path $d) { Remove-Item -Recurse -Force $d }
git clone https://github.com/swimhigh/roc_desk-explorer.git $d
New-Item -ItemType Directory -Force "$d/src-web/src/components/Explorer", "$d/src-web/src/components/LocalExplorer", "$d/src-web/src/hooks", "$d/src-web/src/stores", "$d/src-web/src/services" | Out-Null
Copy-Item 'src-web/src/components/Explorer/*' "$d/src-web/src/components/Explorer/"
Copy-Item 'src-web/src/components/LocalExplorer/*' "$d/src-web/src/components/LocalExplorer/"
Copy-Item 'src-web/src/hooks/useFileTreeOperations.ts' "$d/src-web/src/hooks/"
Copy-Item 'src-web/src/stores/explorerStore.ts' "$d/src-web/src/stores/"
Copy-Item 'src-web/src/stores/localFsStore.ts' "$d/src-web/src/stores/"
Copy-Item 'src-web/src/services/fsService.ts' "$d/src-web/src/services/"
Copy-Item 'src-web/src/services/localFsService.ts' "$d/src-web/src/services/"
@'
# roc_desk-explorer migration

The explorer and local explorer UI, stores, filesystem services, and shared
file-tree operation hook are now tracked here as the canonical migration
source. The host keeps a compatibility copy until the `FileTreeBackend` and
filesystem IPC adapters are published by `roc_desk-common`.

Next: extract the backend command adapter and add the standalone shell.
'@ | Set-Content "$d/MIGRATION.md" -Encoding UTF8
Push-Location $d
git config user.name 'lipeng'
git config user.email 'lipeng@szkingdom.com'
git add .
git commit -m 'Migrate explorer frontend and filesystem adapters' | Out-Null
git push origin main
Pop-Location
Write-Output $d
