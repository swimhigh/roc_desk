$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-http-migrate'
if (Test-Path $d) { Remove-Item -Recurse -Force $d }
git clone https://github.com/swimhigh/roc_desk-http.git $d
New-Item -ItemType Directory -Force "$d/lib/src/http_desk", "$d/src-web/src/components/HttpDesk", "$d/src-web/src/services" | Out-Null
Copy-Item -Recurse 'src-tauri/src/http_desk/*' "$d/lib/src/http_desk/"
Copy-Item 'src-tauri/src/commands/http_desk.rs' "$d/lib/src/http_desk_commands.rs"
Copy-Item 'src-web/src/components/HttpDesk/*' "$d/src-web/src/components/HttpDesk/"
Copy-Item 'src-web/src/services/httpDeskService.ts' "$d/src-web/src/services/"
@'
# roc_desk-http migration

The HTTP workspace implementation has been copied here as the first tool
migration. The canonical implementation is now under `lib/src/http_desk` and
`src-web/src/components/HttpDesk`; the host still retains a compatibility copy
until this crate has its host adapters and standalone Tauri shell.

## Remaining extraction work

1. Replace `crate::` host references in the command adapter with the published
   `roc_desk-common` core interfaces.
2. Add the `lib` Cargo package and `standalone` Tauri shell.
3. Point `roc_desk` at a tagged release and remove its compatibility copy.
'@ | Set-Content "$d/MIGRATION.md" -Encoding UTF8
@'
# HTTP tool source package (phase 1 migration)

The source files are kept close to the host layout while dependency adapters
are extracted.
'@ | Set-Content "$d/lib/README.md" -Encoding UTF8
Push-Location $d
git config user.name 'lipeng'
git config user.email 'lipeng@szkingdom.com'
git add .
git commit -m 'Migrate HTTP workspace source and frontend components' | Out-Null
git push origin main
Pop-Location
Write-Output $d
