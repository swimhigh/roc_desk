$ErrorActionPreference = 'Stop'
$tools = @(
  @{ Name = 'roc_desk-http'; Package = 'roc_desk_http_standalone' },
  @{ Name = 'roc_desk-explorer'; Package = 'roc_desk_explorer_standalone' },
  @{ Name = 'roc_desk-editor'; Package = 'roc_desk_editor_standalone' },
  @{ Name = 'roc_desk-sql'; Package = 'roc_desk_sql_standalone' },
  @{ Name = 'roc_desk-ssh'; Package = 'roc_desk_ssh_standalone' },
  @{ Name = 'roc_desk-workspace'; Package = 'roc_desk_workspace_standalone' }
)
foreach ($tool in $tools) {
  $d = Join-Path $env:TEMP ($tool.Name + '-shell')
  if (-not (Test-Path $d)) { git clone ("https://github.com/swimhigh/{0}.git" -f $tool.Name) $d }
  if ($tool.Name -eq 'roc_desk-http') {
    $manifest = Join-Path $d 'lib/Cargo.toml'
    $fixed = (Get-Content $manifest -Raw).Replace('`r`n', "`r`n")
    Set-Content $manifest $fixed -Encoding UTF8
  }
  @"
param(
  [ValidateSet('debug','release')]
  [string]`$Configuration = 'release'
)
`$ErrorActionPreference = 'Stop'
`$root = Split-Path -Parent `$MyInvocation.MyCommand.Path
`$profile = if (`$Configuration -eq 'release') { '--release' } else { '' }
if (`$profile) { cargo build --workspace --release } else { cargo build --workspace }
`$suffix = if (`$Configuration -eq 'release') { 'release' } else { 'debug' }
`$source = Join-Path `$root ("target/{0}/$($tool.Package).exe" -f `$suffix)
if (-not (Test-Path `$source)) { throw "Build did not produce `$source" }
`$bin = Join-Path `$root 'bin'
New-Item -ItemType Directory -Force `$bin | Out-Null
`$destination = Join-Path `$bin '$($tool.Name).exe'
Copy-Item `$source `$destination -Force
Write-Output "Built `$destination"
"@ | Set-Content (Join-Path $d 'build-standalone.ps1') -Encoding UTF8
  @"
# $($tool.Name) standalone build

Run from the repository root:

```powershell
.\build-standalone.ps1
```

The release executable is written to `bin\$($tool.Name).exe`. Use
`-Configuration debug` for a fast development build.
"@ | Set-Content (Join-Path $d 'BUILD.md') -Encoding UTF8
  Push-Location $d
  git add build-standalone.ps1 BUILD.md
  git commit -m 'Add direct standalone EXE build script' | Out-Null
  git push origin main
  & .\build-standalone.ps1
  Pop-Location
  Write-Output "BUILT $($tool.Name)"
}
