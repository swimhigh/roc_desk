$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-sql-migrate'
if (Test-Path $d) { Remove-Item -Recurse -Force $d }
git clone https://github.com/swimhigh/roc_desk-sql.git $d
New-Item -ItemType Directory -Force "$d/lib/src/sql", "$d/src-web/src/components/SqlDesk", "$d/src-web/src/stores", "$d/src-web/src/services" | Out-Null
Copy-Item -Recurse 'src-tauri/src/sql/*' "$d/lib/src/sql/"
Copy-Item 'src-web/src/components/SqlDesk/*' "$d/src-web/src/components/SqlDesk/"
Copy-Item 'src-web/src/stores/sqlStore.ts','src-web/src/stores/sqlAgentStore.ts' "$d/src-web/src/stores/"
Copy-Item 'src-web/src/services/sqlService.ts','src-web/src/services/sqlAgentService.ts' "$d/src-web/src/services/"
@'
# roc_desk-sql migration

SQL backend, workspace UI, stores, and services are tracked here as the
canonical migration source. The host copy remains until credential, database,
and AI interfaces are consumed through `roc_desk-common`.
'@ | Set-Content "$d/MIGRATION.md" -Encoding UTF8
Push-Location $d; git config user.name 'lipeng'; git config user.email 'lipeng@szkingdom.com'; git add .; git commit -m 'Migrate SQL workspace source' | Out-Null; git push origin main; Pop-Location
