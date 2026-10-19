$ErrorActionPreference = 'Stop'
$d = Join-Path $env:TEMP 'roc_desk-http-migrate'
@'
[package]
name = "roc_desk_http"
version = "0.1.0"
edition = "2021"

[features]
default = []
host-adapter = []

[dependencies]
roc_desk_core = { git = "https://github.com/swimhigh/roc_desk-common", package = "roc_desk_core", tag = "core-v0.1.0" }
async-trait = "0.1"
base64 = "0.22"
chrono = { version = "0.4", features = ["serde"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "multipart"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
thiserror = "2"
uuid = { version = "1", features = ["v4", "serde"] }
'@ | Set-Content "$d/lib/Cargo.toml" -Encoding UTF8
@'
//! HTTP workspace domain modules.
pub mod client;
pub mod import;
pub mod model;
pub mod vars;
#[cfg(feature = "host-adapter")]
pub mod export;
#[cfg(feature = "host-adapter")]
pub mod service;
'@ | Set-Content "$d/lib/src/http_desk/mod.rs" -Encoding UTF8
@'
//! HTTP domain logic.
#[path = "http_desk/mod.rs"]
pub mod http_desk;
pub mod error { pub use roc_desk_core::error::AppError; }
pub use roc_desk_core::error::AppError;
pub use http_desk::{client, import, model, vars};
'@ | Set-Content "$d/lib/src/lib.rs" -Encoding UTF8
@'
node_modules/
target/
Cargo.lock
'@ | Set-Content "$d/.gitignore" -Encoding UTF8
Push-Location $d
Remove-Item -Recurse -Force target -ErrorAction SilentlyContinue
cargo check --workspace
git add -A
git commit -m 'Make HTTP core package compile without host adapters' | Out-Null
git push origin main
Pop-Location
