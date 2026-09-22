# Multi-repository split progress

This file records the repository-local part of phase 0 from
[`MULTI_REPO_SPLIT_PLAN.md`](./MULTI_REPO_SPLIT_PLAN.md).

## Completed

- The Rust workspace now contains `src-tauri/core` (`roc_desk_core`) and
  `src-tauri/common` (`roc_desk_common`) alongside the host application.
- `AppError` lives in `roc_desk_core` and the host keeps a compatibility
  re-export while the remaining modules are migrated.
- The JavaScript workspace has package entry points for `@roc_desk/ui-core`
  and `@roc_desk/common-web`. Their source bridges keep the current host build
  working while components and bindings move incrementally.
- The host remains the only executable package; neither common package has a
  Tauri entry point or a reverse dependency on the host.
- The public `roc_desk-common` repository has been initialized and tagged:
  `core-v0.1.0`, `common-v0.1.0`, `ui-core-v0.1.0`, and `common-web-v0.1.0`.
- The first tool migration has started in `roc_desk-http`; its HTTP backend,
  command adapter, frontend components, and service layer are now tracked in
  that repository while the host keeps its compatibility copy.
- Initial source migrations are now present in all six tool repositories:
  `roc_desk-http`, `roc_desk-explorer`, `roc_desk-editor`, `roc_desk-sql`,
  `roc_desk-ssh`, and `roc_desk-workspace`.

## Migration rule

New shared Rust code goes in `roc_desk_core` (required primitives) or
`roc_desk_common` (optional cross-tool services). New shared frontend code is
exported through the package entry points. Tool code must not import another
tool through a host-internal path. A module is removed from the host only
after the workspace build and frontend build both pass.

## Verification

```text
cargo check --workspace
npm run build:web
```

Both commands pass at the current phase-0 boundary.

The HTTP tool is intentionally not wired into the host as a Git dependency yet:
its command adapter still references host-only `AppState`, workspace handles,
and filesystem traits. That adapter must be replaced by the common interfaces
before deleting the host copy.

The same compatibility rule applies to the other tools. Their copied source is
now canonical in the tool repositories, but host deletion waits for an
independent Cargo/npm package build and a standalone shell check.

The HTTP repository now has a compiling independent Cargo workspace with
`lib` and `standalone` members. The core HTTP logic builds against the tagged
`roc_desk-common` dependency; host filesystem/database adapters remain behind
the `host-adapter` feature until their public interfaces are finalized.

The other five tool repositories now have the same compiling `lib` plus
`standalone` Cargo workspace shell, each consuming `roc_desk_core` at
`core-v0.1.0`. Their migrated business sources remain alongside the shell and
will be wired into the library crate as host adapters are extracted.

All six tool repositories now provide `build-standalone.ps1`. Running it from
the repository root performs a Release Cargo build and copies the executable to
`bin/<tool>.exe`; `-Configuration debug` is available for development builds.
Each tool README is written in Chinese and documents its purpose, dependencies,
build command, migration status, and the reserved `docs/screenshots/` location.

The host now consumes `roc_desk_core` and `roc_desk_common` directly from the
public `roc_desk-common` repository tags; the local phase-0 copies are no
longer workspace members. The host `cargo check --workspace` passes with this
cross-repository dependency graph.

Each tool repository also has a GitHub Actions workflow that builds its EXE on
Windows, uploads it as an artifact, and attaches it to a Release when a
`vX.Y.Z` tag is pushed. The release-only repository remains reserved for the
future combined distribution flow.

The first combined manual-test release is available at
`roc_desk-releases` tag `v0.1.0`, with all six EXE assets and SHA-256 values.
These binaries are currently standalone migration shells; functional tool
behavior will be published after each tool's business modules are wired into
its standalone Tauri host.

## Explorer (资源管理器)：local filesystem surface fully migrated (2026-09-22)

- `roc_desk_common::fsops::FileOps` in `roc_desk-common` (tag `common-v0.3.0`)
  expanded from the 5-method placeholder to the full contract used by the
  host: `read_file_raw`, `file_size`, `read_file_raw_bounded`,
  `write_file_bytes`, `rename`, `copy`, plus the editor/binary-preview
  helpers (`read_bytes_for_editor`, `read_file_for_editor`,
  `read_binary_for_preview`, `download_to_local_file`). `LocalFileOps` fully
  implements it. Also moved as new pure-logic shared modules: `encoding`
  (UTF-8/GBK/UTF-16 detection), `binary_info` (PE/ELF/Mach-O inspection),
  `jar_info` (JAR/manifest inspection), `office_convert` (legacy Office→PDF
  via LibreOffice). Not moved: `search_stream`, `copy_between`,
  `open_path_or_launch_exe` — these need `tauri::AppHandle`/`Emitter` and
  don't belong in a host-agnostic crate; `open_path_or_launch_exe` was
  reimplemented directly in the explorer `lib` crate instead.
- `roc_desk-explorer` (tag `v0.2.0`) now implements all 19 local-filesystem
  commands from the host's old `commands/local_fs.rs` under
  `roc_desk_explorer::cmd::*` (a submodule, not crate root — Tauri's command
  macro emits both a `#[macro_export]` macro and a self-referential `pub use`
  of the same name, which collide with `E0255` at crate root). Remote
  (SFTP/Windows Agent) browsing is intentionally NOT part of this crate —
  it overlaps with `roc_desk-ssh` and stays pending until that tool's split.
- The host now depends on `roc_desk_explorer` at `v0.2.0` and registers its
  19 commands via `roc_desk_explorer::cmd::*` in `lib.rs`.
  `commands/local_fs.rs` was slimmed down to just `take_pending_open_paths`
  (reads host-only `AppState.pending_open_paths`, has no meaning outside the
  host). `cargo check --workspace` and a full `build-portable.ps1` run both
  pass; `bin/roc_desk.exe` builds successfully with the new dependency graph.
- `roc_desk-explorer/src-web/` still has no `package.json`/Vite scaffold of
  its own (only migrated `.tsx`/`.ts` source files) — its standalone exe
  currently ships a placeholder `dist/index.html`. Wiring a real frontend
  build for the standalone shell is a separate task, not required for the
  host integration above (the host's own frontend already has the real
  Explorer UI and is unaffected).
- Remaining before explorer can be considered fully done: remote SFTP/Agent
  commands (currently still in the host, `commands/sftp.rs` /
  `commands/agent.rs`) need to be resolved together with the `roc_desk-ssh`
  migration, since they overlap.
