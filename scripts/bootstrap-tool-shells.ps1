$ErrorActionPreference = 'Stop'
$tools = @(
  @{ Name = 'roc_desk-explorer'; Package = 'roc_desk_explorer'; Description = 'Resource explorer' },
  @{ Name = 'roc_desk-editor'; Package = 'roc_desk_editor'; Description = 'Standalone editor' },
  @{ Name = 'roc_desk-sql'; Package = 'roc_desk_sql'; Description = 'SQL workspace' },
  @{ Name = 'roc_desk-ssh'; Package = 'roc_desk_ssh'; Description = 'SSH SFTP Agent and RDP' },
  @{ Name = 'roc_desk-workspace'; Package = 'roc_desk_workspace'; Description = 'Coding workspace' }
)
foreach ($tool in $tools) {
  $d = Join-Path $env:TEMP ($tool.Name + '-shell')
  if (Test-Path $d) { Remove-Item -Recurse -Force $d }
  git clone ("https://github.com/swimhigh/{0}.git" -f $tool.Name) $d
  New-Item -ItemType Directory -Force "$d/lib/src", "$d/standalone/src" | Out-Null
  @"
[workspace]
resolver = "2"
members = ["lib", "standalone"]
"@ | Set-Content "$d/Cargo.toml" -Encoding UTF8
  @"
[package]
name = "$($tool.Package)"
version = "0.1.0"
edition = "2021"

[dependencies]
roc_desk_core = { git = "https://github.com/swimhigh/roc_desk-common", package = "roc_desk_core", tag = "core-v0.1.0" }
"@ | Set-Content "$d/lib/Cargo.toml" -Encoding UTF8
  @"
//! $($tool.Description) library boundary.
//! Business source is migrated under `src/` and is wired into this crate as
//! host adapters are replaced with roc_desk-common interfaces.
pub const TOOL_NAME: &str = "$($tool.Name)";
"@ | Set-Content "$d/lib/src/lib.rs" -Encoding UTF8
  @"
[package]
name = "$($tool.Package)_standalone"
version = "0.1.0"
edition = "2021"

[dependencies]
$($tool.Package) = { path = "../lib" }
"@ | Set-Content "$d/standalone/Cargo.toml" -Encoding UTF8
  @"
fn main() {
    println!("$($tool.Name) standalone shell");
}
"@ | Set-Content "$d/standalone/src/main.rs" -Encoding UTF8
  @'
target/
Cargo.lock
'@ | Set-Content "$d/.gitignore" -Encoding UTF8
  Push-Location $d
  cargo check --workspace
  git add -A
  git commit -m 'Add independent library and standalone shell' | Out-Null
  git push origin main
  Pop-Location
  Write-Output "BOOTSTRAPPED $($tool.Name)"
}
