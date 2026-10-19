$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-http-migrate'
@'
[workspace]
resolver = "2"
members = ["lib", "standalone"]
'@ | Set-Content "$d/Cargo.toml" -Encoding UTF8
New-Item -ItemType Directory -Force "$d/lib/src", "$d/standalone/src" | Out-Null
@'
[package]
name = "roc_desk_http"
version = "0.1.0"
edition = "2021"

[dependencies]
roc_desk_core = { git = "https://github.com/swimhigh/roc_desk-common", package = "roc_desk_core", tag = "core-v0.1.0" }
async-trait = "0.1"
base64 = "0.22"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
uuid = { version = "1", features = ["v4", "serde"] }
'@ | Set-Content "$d/lib/Cargo.toml" -Encoding UTF8
@'
//! HTTP domain logic. Host-specific filesystem and database adapters are
//! intentionally kept behind the `host-adapter` feature during migration.
#[path = "http_desk/mod.rs"]
pub mod http_desk;
pub use roc_desk_core::error::AppError;
pub use http_desk::{client, import, model, vars};
'@ | Set-Content "$d/lib/src/lib.rs" -Encoding UTF8
@'
[features]
default = []
host-adapter = []
'@ | Add-Content "$d/lib/Cargo.toml" -Encoding UTF8
@'
[package]
name = "roc_desk_http_standalone"
version = "0.1.0"
edition = "2021"

[dependencies]
roc_desk_http = { path = "../lib" }
'@ | Set-Content "$d/standalone/Cargo.toml" -Encoding UTF8
@'
fn main() {
    println!("roc_desk-http standalone shell (migration scaffold)");
}
'@ | Set-Content "$d/standalone/src/main.rs" -Encoding UTF8
Push-Location $d
cargo check --workspace
git add .
git commit -m 'Add independent HTTP library and standalone shell' | Out-Null
git push origin main
Pop-Location
