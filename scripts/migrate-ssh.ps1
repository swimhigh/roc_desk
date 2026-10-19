$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-ssh-migrate'
if (Test-Path $d) { Remove-Item -Recurse -Force $d }
git clone https://github.com/swimhigh/roc_desk-ssh.git $d
New-Item -ItemType Directory -Force "$d/lib/src/ssh", "$d/lib/src/sftp", "$d/lib/src/agent", "$d/lib/src/rdp", "$d/src-web/src/components/RemoteTool", "$d/src-web/src/components/SftpBrowser", "$d/src-web/src/stores", "$d/src-web/src/services" | Out-Null
Copy-Item -Recurse 'src-tauri/src/ssh/*' "$d/lib/src/ssh/"
Copy-Item -Recurse 'src-tauri/src/agent/*' "$d/lib/src/agent/"
Copy-Item -Recurse 'src-tauri/src/rdp/*' "$d/lib/src/rdp/"
Copy-Item 'src-web/src/components/RemoteTool/*' "$d/src-web/src/components/RemoteTool/"
Copy-Item 'src-web/src/components/SftpBrowser/*' "$d/src-web/src/components/SftpBrowser/"
Copy-Item 'src-web/src/stores/sftpStore.ts','src-web/src/stores/remoteSessionStore.ts' "$d/src-web/src/stores/"
Copy-Item 'src-web/src/services/sshService.ts','src-web/src/services/sftpService.ts','src-web/src/services/rdpService.ts','src-web/src/services/agentService.ts' "$d/src-web/src/services/"
@'
# roc_desk-ssh migration

SSH/SFTP, Windows Agent, and RDP source has been copied here. Host adapters
remain until connection, credentials, and filesystem contracts are extracted
to `roc_desk-common`; the standalone shell is added before host deletion.
'@ | Set-Content "$d/MIGRATION.md" -Encoding UTF8
Push-Location $d; git config user.name 'lipeng'; git config user.email 'lipeng@szkingdom.com'; git add .; git commit -m 'Migrate SSH SFTP Agent and RDP source' | Out-Null; git push origin main; Pop-Location
