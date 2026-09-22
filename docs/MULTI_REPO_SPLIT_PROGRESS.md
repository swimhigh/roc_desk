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
- Follow-up (2026-09-22): `roc_desk-explorer` shipped a real standalone
  frontend (tag `v0.2.1`) replacing the placeholder `dist/index.html` — a
  self-contained single-pane local file manager (navigate/drives/CRUD/open),
  not a copy of the host's `LocalExplorerScreen.tsx` (which pulls in SFTP/
  Agent/terminal/shared-UI dependencies that don't exist in this repo yet).

## Editor (编辑器)：OCR + symbol-index-for-standalone-mode migrated (2026-09-22)

- `roc_desk-common` (tag `common-v0.3.1`, the current head — supersedes
  `common-v0.4.0`, which is an earlier commit on the same branch; both exist
  as tags because two migrations tagged in parallel, see "known tag-ordering
  quirk" below) gained `roc_desk_core::credential` (`CredentialStore` +
  `KeyringStore`), `roc_desk_core::db` (`DbPool`/`create_pool`/
  `apply_migrations`), and `roc_desk_common::symbols` (the regex-based
  per-language symbol scanner, ported from the host's `symbols/mod.rs`).
- `roc_desk-editor` (tag `v0.2.1`) now owns two genuinely editor-specific
  command groups: `ocr::editor_ocr_image` (Windows `Media.Ocr`, ported 1:1)
  and `symbols::editor_symbols_*` (a **new**, root-path-keyed variant of
  symbol indexing for standalone/loose-folder editing — the host's existing
  `commands/symbols.rs` is `workspace_id`-keyed and stays host-side, see
  below). Local file I/O is not reimplemented — `roc_desk-editor` depends on
  `roc_desk-explorer` and re-exports it. A missing `src-web` build scaffold
  was filled in (package.json/vite/tsconfig) and `vite.config.ts`'s
  `build.outDir` was pointed at `../standalone/dist` so `npm run build`
  actually feeds the Tauri shell instead of leaving the placeholder in
  place; `<EditorPane/>` + `useEditorStore` are exported from
  `src-web/src/index.ts` for `roc_desk-workspace` to depend on later.
- **Host wiring done**: `src-tauri/Cargo.toml` now depends on
  `roc_desk_editor` (`v0.2.1`); `lib.rs` registers
  `roc_desk_editor::ocr::editor_ocr_image` in place of the deleted
  `commands/ocr.rs`. `cargo check --workspace` and a full
  `build-portable.ps1` run both pass; smoke-tested by launching
  `bin/roc_desk.exe` (stayed up, no crash).
- **Host wiring NOT done, intentionally**: the host's `commands/symbols.rs`
  (`symbols_build_index`/`symbols_go_to_definition`/`symbols_reindex_file`)
  was **not** switched to call `roc_desk_common::symbols::build_index`. The
  host's `WorkspaceHandle.file_ops` is `Arc<dyn crate::fsops::FileOps>` —
  the host's own trait, which has one more method (`replace_text`) than
  `roc_desk_common::fsops::FileOps` and is therefore a *different, nominally
  incompatible* trait object even though most methods match structurally.
  Passing `handle.file_ops.as_ref()` into `roc_desk_common::symbols::
  build_index` (which expects `&dyn roc_desk_common::fsops::FileOps`) does
  not type-check. Unifying the two traits (e.g. making host's `FileOps`
  re-export/extend `roc_desk_common`'s, or moving `replace_text` out to a
  separate extension trait) is real work belonging to a future pass — likely
  when `roc_desk-workspace` (which owns the *actual* caller of these
  workspace-keyed commands) gets migrated, since that's when the host's
  remote/local `fsops` split needs to be resolved anyway. Host's local
  `symbols/mod.rs` (360 lines) was therefore **not deleted** — it's now a
  duplicate of `roc_desk_common::symbols`, left in place deliberately.
- **Known tag-ordering quirk**: `roc_desk-common`'s semver tags are not in
  chronological order — `common-v0.4.0` (editor's parallel push, adds only
  `symbols`) was tagged *before* `common-v0.3.1` (SQL's parallel push, adds
  `credential`+`db` on top of `symbols`) landed and was tagged with a lower
  number. `common-v0.3.1` is the actual latest/superset commit; the host and
  any future work should treat `common-v0.3.1` as current, not `v0.4.0`.
  This was caused by two tool migrations running concurrently and each
  tagging from their own vantage point — worth renumbering
  (e.g. retro-tag the true chronology as `v0.5.0`) before it causes real
  confusion, not urgent since both tags still resolve to valid, buildable
  commits.
## SQL 工作台：tool-repo side done, host wiring deliberately deferred (2026-09-22)

- `roc_desk-common` gained `roc_desk_core::credential` (`CredentialStore` +
  `KeyringStore`) and `roc_desk_core::db` (`DbPool`/`create_pool`/
  `apply_migrations`), tagged `common-v0.3.1` (see the tag-ordering note
  above — this is the true latest, not `v0.4.0`).
- `roc_desk-sql` (tag `v0.2.0`) has all 35 non-AI commands
  (`sql_list_data_sources` ... `sql_write_text_file`, matching the host's
  `commands/sql.rs` registration list exactly) ported to `roc_desk_sql::cmd`,
  backed by a self-contained `SqlAppState` (data source service, session
  manager, executor, query history, workspace tabs, workspace cache,
  transfer manager — all newly built for this crate, no host `AppState`
  precedent existed since explorer's commands are stateless). MySQL/
  PostgreSQL/SQL Server adapters compile and are wired (Oracle stays
  "not implemented", matching the host). `cargo check`/`build --release`
  both pass; a smoke-test integration test does a real insert/select round
  trip against the tool's own SQLite metadata file.
- **Deliberately NOT ported**: the SQL Agent AI chat loop
  (`sql::agent`/`commands/sql_agent.rs`) and the one-shot AI assist panel
  (`sql::ai_assistant`/`sql_ai_*`/`sql_accept_change`/etc.) — both need
  host-only `crate::ai`/`crate::agent_llm`/`crate::coding::ChangeStore`
  infrastructure that has no `roc_desk_core` equivalent yet.
- **Host wiring NOT done — this is a real blocker, not just unfinished
  busywork, read before attempting it**: unlike explorer/editor, the 35
  ported commands **cannot** simply be pointed at `roc_desk_sql::SqlAppState`
  while leaving host's own `AppState.sql_data_source_service`/
  `sql_session_manager`/`sql_query_history` fields in place, because
  `commands/sql_agent.rs` (which stays in the host — the multi-turn AI Agent
  chat, 13 usages concentrated in `sql/agent/session.rs` +
  `commands/sql_agent.rs`) reads those exact host-typed fields. Swapping
  only the 35 commands over would silently split "data sources visible in
  the SQL workbench UI" from "data sources visible to the SQL Agent chat"
  into two independent registries backed by two different SQLite files —
  a user's newly-added data source would work in the workbench but be
  invisible to the AI Agent. That is a functional regression, not a
  refactor, so it was not done.
  **The correct fix** (scoped follow-up, not attempted this pass): point
  `roc_desk_sql::SqlAppState::new` at the host's *existing*
  `sql_data_sources.db`/main pool path (no data migration needed, same
  file) rather than a separate file, register it as the single source of
  truth, then update `sql/agent/session.rs` and `commands/sql_agent.rs`
  (13 usages) to read `State<'_, roc_desk_sql::SqlAppState>` fields instead
  of `state.sql_data_source_service`/`sql_session_manager`/
  `sql_query_history` — after which host's own `sql/service.rs`,
  `db/repo/sql_data_sources_repo.rs`, `db/repo/sql_query_history_repo.rs`,
  `db/repo/sql_workspace_tabs_repo.rs`, `sql/executor.rs`,
  `sql/workspace_cache.rs`, `sql/transfer.rs`, and the 35 handlers in
  `commands/sql.rs` can be deleted. `sql/agent/`, `sql/ai_assistant.rs`, and
  the AI-only command handlers in `commands/sql.rs` stay in the host
  regardless, until `roc_desk_core::ai` exists for them to be ported onto.
- **Frontend**: not started, same blocker explorer/SQL both hit — the SQL
  frontend imports heavily from host-only shared components (`ResultPanel`,
  `CodingAgent/*`, `AiChat/*`, `shared/Toast|ContextMenu|ConfirmDialog`)
  that haven't been extracted into `roc_desk-common`'s `ui-core` package yet
  (currently just a bindings stub, not real components).

- **Known cross-tool dependency-graph quirk**: the host currently pulls
  *two different commits* of `roc_desk_core`/`roc_desk_common` into the same
  build — one directly (pinned to `common-v0.3.1`), one transitively via
  `roc_desk-explorer` (pinned to `common-v0.3.0`) and another via
  `roc_desk-editor` (pinned to `common-v0.4.0`). This compiles today because
  no code currently passes a value of one instantiation's types (e.g.
  `AppError`) across a boundary that expects the other instantiation's
  types — each tool crate's commands are self-contained and only meet the
  host via Tauri's serde-serialized IPC boundary, not shared Rust type
  identity. It is still fragile and should be cleaned up by bumping
  `roc_desk-explorer`/`roc_desk-editor` to both pin `common-v0.3.1` next
  time either is touched, rather than left to accumulate further.
