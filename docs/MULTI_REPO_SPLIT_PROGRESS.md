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
## SQL 工作台：host wiring done (2026-09-24)

- Resolved the blocker described below: `roc_desk_sql::SqlAppState::new` is
  now pointed at the host's *existing* `db_path` (the same SQLite file
  everything else uses), not a separate file — confirmed byte-identical
  schema between `roc_desk-sql`'s `migrations/0001_sql_desktop.sql` and the
  host's `migrations/0020_sql_desktop.sql` first (`diff` showed zero output),
  then seeded `schema_migrations` with a row for `'0001_sql_desktop'` before
  constructing `SqlAppState` so `roc_desk_core::db::migrate::apply_migrations`
  doesn't try to re-run `CREATE TABLE sql_data_sources` (no `IF NOT EXISTS`
  in that migration) against a database where the table already exists under
  a different migration name.
- `commands/sql.rs` now only contains the AI panel (`sql_ai_generate/explain/
  optimize/fix_error`, `sql_accept/reject/undo_change`, `sql_revert_turn`) —
  the 29 non-AI commands are registered as `roc_desk_sql::cmd::*` in
  `lib.rs`. The AI panel's two helpers (`get_or_create_sql_change_store`,
  `stage_ai_result`) and `commands/sql_agent.rs`'s functions that touched
  `state.sql_data_source_service`/`sql_session_manager`/`sql_query_history`
  now take an additional `State<'_, roc_desk_sql::SqlAppState>` parameter and
  read `sql_state.data_source_service`/`session_manager`/`query_history`/
  `workspace_tabs`/`workspace_cache` instead — Tauri natively supports a
  command taking multiple different `State<T>` parameters, no extension-slot
  mechanism needed. `sql/agent/session.rs`'s imports were repointed from
  `crate::sql::{adapter,model,policy,service}`/
  `crate::db::repo::sql_query_history_repo` to the `roc_desk_sql::` 
  equivalents (function signatures there already took these types as
  parameters rather than reading `state.field` directly, so no other changes
  were needed there).
- Host's `state.rs` no longer has `sql_data_source_service`/
  `sql_session_manager`/`sql_query_history`/`sql_workspace_tabs`/
  `sql_workspace_cache`/`sql_executor`/`sql_transfer_manager` fields (kept
  `sql_changes`/`sql_ai_assistant`/the `sql_agent_*` fields, which stay
  host-only). Deleted the now-fully-migrated
  `sql/{adapter,adapters/,data_editor,executor,model,policy,registry,
  service,transfer,workspace_cache}.rs` and
  `db/repo/sql_{data_sources,query_history,workspace_tabs}_repo.rs`; kept
  `sql/agent/` and `sql/ai_assistant.rs`.
- `cargo check --release` (zero warnings) and a full `build-portable.ps1` run
  both pass; smoke-tested by launching `bin/roc_desk.exe` directly (process
  stayed alive, empty error log, cleanly killed afterward).
- Depends on the same-day tag-alignment fix (`common-v0.5.0` unified across
  all consumers, see the top of `docs/MULTI_REPO_SPLIT_PLAN.md` §13) — before
  that fix, `roc_desk-sql`'s `AppError`/`DbPool` types would have been a
  *different* Rust type identity than the host's own (pinned to a different
  `roc_desk-common` git tag), which would have made this exact
  multi-`State<T>` wiring fail to type-check.

## SQL 工作台（tool-repo side）：tool-repo side done, host wiring deliberately deferred (2026-09-22)

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

## HTTP 测试工作台：host wiring done (2026-09-22)

- `roc_desk-http`'s command layer was a dead file (`http_desk_commands.rs`,
  never `mod`-declared, still referencing host-only `AppState`/
  `WorkspaceHandle`). Rewrote it as root-path-keyed (`root: String` instead
  of `workspace_id: Uuid`, mirroring `roc_desk-editor`'s `editor_symbols_*`
  precedent) inside `lib.rs`'s `pub mod cmd`, backed by a self-contained
  `HttpAppState` (its own SQLite file for request history/tabs, built on
  `roc_desk_core::db`). Also fixed real encoding corruption in
  `export.rs`/`service.rs` (bare `\r` bytes inside doc comments broke
  rustc's doc-comment parser and silently merged adjacent logical lines —
  not a cosmetic issue, an actual `fn` declaration had gotten spliced onto
  the end of a doc comment) by restoring clean copies from the host and
  reapplying the two import-path changes. Tagged **`v0.2.0`**, pushed.
- **Host wiring done**: `src-tauri/Cargo.toml` depends on `roc_desk_http`
  (`v0.2.0`). `commands/http_desk.rs`, `http_desk/` (the whole module),
  `db/repo/http_request_history_repo.rs`, and
  `db/repo/http_workspace_tabs_repo.rs` are deleted; the 28 commands are
  registered as `roc_desk_http::cmd::*`; `AppState` no longer carries
  `http_workspace_tabs`/`http_request_history` (those two tables were
  FK'd to `workspaces(id)` — the new `HttpAppState` has no such FK, it's
  keyed by `root: String` directly, so it gets its own SQLite file
  `<app_data_dir>/http_desk.db` instead of sharing `workspaces_pool`).
  Existing rows in the old FK'd tables are orphaned (not deleted, just
  unread going forward) — acceptable, this is disposable request-history
  cache data, not credentials or connection profiles.
  `cargo check --release` and a full `build-portable.ps1` run both pass;
  smoke-tested by launching `bin/roc_desk.exe` (stayed up, no crash).
- **Environment note for future verification passes**: `cargo check`
  (debug profile) on this host is currently blocked by 360 Security
  quarantining a freshly-compiled `num-bigint-dig` build script
  (`拒绝访问`/os error 5) every time `build/debug` doesn't already have a
  trusted copy — this is unrelated to any of this session's code changes
  (it's a transitive dependency of the SSH/RSA stack). Retrying doesn't
  help (confirmed identical failure across 6 attempts with 20s waits).
  **Workaround that does work**: `cargo check --release` (or any
  `--release` build) reuses `build/release`, which already has a
  360-trusted copy from an earlier successful `build-portable.ps1` run —
  use that instead of plain `cargo check` until someone adds a proper 360
  exclusion for `F:\code\wuyou\roc_desk\build\` (or wherever
  `CARGO_TARGET_DIR` points).

## SSH/SFTP/RDP/Agent：host wiring done (2026-09-24)

- Resolved the blocker described below using the same pattern as the SQL
  wiring: `RocDeskSshAppState::new` points at the host's *existing*
  `sessions_db_path` (not a new file). Confirmed the `connections`/
  `connection_groups`/`known_hosts`/`agent_known_hosts` schemas are
  column-for-column identical to `roc_desk-ssh`'s consolidated
  `0001_ssh_init` migration (diffed against the host's own incremental
  `0002_connections`/`0010_connection_protocol`/`0013_agent_known_hosts`),
  then seeded `schema_migrations` with `'0001_ssh_init'` before construction.
  One wrinkle SQL didn't have: the host's `transfer_log` table lived in the
  *main* `roc_desk.db`, not `sessions.db`, but `roc_desk_ssh` expects it
  alongside the connection data — manually created a fresh empty
  `transfer_log` table in `sessions.db` (matching `roc_desk_ssh`'s schema)
  as part of the same seed step; old transfer history in `roc_desk.db` is
  orphaned (same acceptable trade-off as the HTTP migration's history table).
- `commands/coding.rs` (`build_new_session` + 3 callers, `coding_send_message`,
  `coding_set_auto_git_commit`, `maybe_auto_continue` + 2 callers),
  `commands/sql.rs` (`stage_ai_result` + 3 callers, `sql_accept_change`), and
  `commands/log_search.rs` (`log_search_live`, `log_import_remote_paths`) now
  take an additional `State<'_, roc_desk_ssh::RocDeskSshAppState>` parameter
  wherever they read `ssh_pool`/`agent_pool`/`connection_manager` — same
  multi-`State<T>` pattern as the SQL migration, no new mechanism needed.
  `coding/changes.rs`/`coding/git_ops.rs`/`coding/session.rs`/
  `workspace/mod.rs`/`log/remote.rs`/`log/importer.rs`/`fsops/agent.rs`/
  `fsops/remote.rs` had their `SshConnectionPool`/`AgentConnectionPool`/
  `ConnectionManager`/`Protocol`/`SshSession`/`RemoteFileOps`/`FileOps`
  imports repointed to `roc_desk_ssh::{ssh,agent,connection,fsops}` (these
  take the types as parameters rather than reading `state.field`, so mostly
  import-path swaps — `log/importer.rs`'s `import_remote_paths`/
  `expand_remote_paths`/`download_and_import_remote` needed their
  `RemoteFileOps` typed against `roc_desk_ssh`'s copy specifically because
  they call its SFTP-specific inherent `download_to_local`, which isn't part
  of the generic `FileOps` trait).
- **Found and fixed a real type-split bug, not just a theoretical risk**:
  `roc_desk-ssh` vendors its own copy of the `roc_desk_protocol` wire-format
  crate (documented in its own module doc as intentional — the standalone
  `roc_desk_agent.exe` binary isn't part of this migration). The host's
  `src-tauri/Cargo.toml` still had `roc_desk_protocol = { path = "../protocol" }`
  pointing at the *local* copy — a different SourceId than what
  `roc_desk_ssh::agent::session::AgentSession` was compiled against, so
  `fsops/agent.rs`/`coding/session.rs` (same-process Rust code calling into
  `AgentSession::request()`) failed with "expected `Request`, found a
  different `Request`" despite byte-identical source. Fixed by pointing the
  host's `roc_desk_protocol` dependency at the *same* git tag `roc_desk-ssh`
  resolves internally (`{ git = "...roc_desk-ssh", package = "roc_desk_protocol",
  tag = "v0.3.1" }`) instead of the local path — `agent/Cargo.toml` (the
  actual `roc_desk_agent.exe` binary deployed to remote servers) keeps the
  local path dependency unchanged, since it's a separate OS process
  communicating over the wire, not sharing Rust type identity with the host.
- Host's `AppState` no longer has `connection_manager`/
  `connection_group_manager`/`ssh_pool`/`rdp_sessions`/`trust_prompts`/
  `agent_pool`/`agent_trust_prompts`/`cancelled_transfers`/`transfer_log`
  (nine fields). Deleted the fully-migrated `ssh/`, `agent/`, `rdp/`,
  `connection/` module directories, `commands/{ssh,sftp,rdp,agent,connection,
  connection_group,transfer}.rs`, and
  `db/repo/{connections,connection_groups,known_hosts,agent_known_hosts,
  transfer_log}_repo.rs`.
- `cargo check --release` (zero warnings) and a full `build-portable.ps1` run
  both pass; smoke-tested by launching `bin/roc_desk.exe` directly (process
  stayed alive ~6s, no new entries in the error log beyond pre-existing
  unrelated ones from a concurrent user session, cleanly killed afterward).
- Explicitly deferred (matches the plan): `commands/coding.rs`'s AI Agent
  loop itself, `commands/symbols.rs`, and `roc_desk-workspace`'s own host
  wiring are unaffected by this pass — `编程工作区` migration is still next.

## SSH/SFTP/RDP/Agent（tool-repo side）：tool-repo side done, host wiring deliberately deferred (2026-09-22)

- `roc_desk-ssh` (tag `v0.3.0`) has SSH terminal (connect/auth/PTY/TOFU),
  SFTP (list/read/write/transfer/preview), Windows remote Agent (TLS+pairing
  auth, cert TOFU, terminal, file browsing), and RDP (launches `wfreerdp.exe`,
  embeds its window via Win32 APIs) all fully ported and wired as ~55
  commands in `roc_desk_ssh::cmd`, backed by its own `RocDeskSshAppState`
  (self-contained SQLite file for connection profiles/known-hosts/transfer
  log — does not depend on `roc_desk_core::workspace`, since that's still a
  placeholder). Verified via a debug build that boots and initializes its
  state layer correctly; release build blocked by the same 360 AV issue
  documented above (debug/`cargo check --release` workaround applies here
  too, not yet exercised for this specific repo).
- **Host wiring NOT done — same class of blocker as SQL, read before
  attempting it**: host's `AppState.connection_manager`/`ssh_pool`/
  `agent_pool`/`trust_prompts`/`agent_trust_prompts`/`rdp_sessions`/
  `connection_group_manager` are read not just by `commands/ssh.rs`/
  `sftp.rs`/`rdp.rs`/`agent.rs`/`connection.rs`/`connection_group.rs` (the
  ones that would be fully replaced by `roc_desk-ssh`), but also by
  `commands/sql.rs` (3 sites), `commands/coding.rs` (7 sites), and
  `commands/log_search.rs` (2 sites) — none of which have been migrated.
  Swapping the connection-management fields over would break those three
  files' still-host-resident commands the same way the SQL Agent chat would
  have broken if the 35 SQL commands had been swapped without it.
  **The correct fix** (future pass): audit exactly what `sql.rs`/`coding.rs`/
  `log_search.rs` need from `connection_manager`/`ssh_pool`/etc.
  (almost certainly: resolving a connection profile by id, and/or reusing an
  already-open SSH tunnel for a remote data source or remote workspace),
  point `roc_desk-ssh`'s `RocDeskSshAppState` at the same underlying data
  (or have host construct it and read the *same* instance from both old and
  new call sites), update those three files' call sites, then swap+delete.
  This likely wants to happen together with the `roc_desk-workspace` host
  wiring below, since `coding.rs` is the connective tissue between them.

## 编程工作区：host wiring partially done — PTY + Git panel only (2026-09-24)

- Wired in `roc_desk_workspace::cmd::pty_open/write/resize/close` (1:1 port
  of host's old `crate::pty`, pure in-memory runtime state, no other host
  code reads `state.local_pty`, so the swap is safe) and the 6 local Git
  panel commands (`git_is_repo/status/diff/log/current_branch/commit_file/
  commit_paths` — pure `cwd`-argument functions, no state at all, and host
  had none of these standalone before this pass, so it's purely additive).
  Deleted host's own `crate::pty` module entirely.
- **Deliberately NOT wired** — same blocker documented in the tool-repo-side
  section below, confirmed still applies: `roc_desk_core::workspace`'s
  `WorkspaceManager` is local-only and has no "currently open workspace"
  registry, vs. host's own `WorkspaceManager`/`state.workspaces` which
  supports remote/SSH workspaces (now wired to `roc_desk_ssh`, see the SSH
  section above) and is read by `commands/coding.rs` (15+ sites) and
  `commands/symbols.rs`. Swapping `workspace_open_local`/`list_recent`/etc.
  for the tool-repo's versions would silently break "opening a workspace
  makes it usable for editing." Host's `commands/workspace.rs` stays
  untouched — it's a strict functional superset. The AI coding-agent loop
  (`coding::session`/`coding::tools`) is unaffected and also untouched, for
  the same `crate::ai`/`crate::agent_llm`-not-yet-extracted reason as before.
- Verification: `cargo check` clean, full `build-portable.ps1` succeeded,
  confirmed the frontend calls `pty_open`/`pty_write`/`pty_resize`/
  `pty_close` by the same unnamespaced command names (`src-web/src/services/
  ptyService.ts`), so the swap is transparent to the UI.

## 编程工作区（tool-repo side）：tool-repo side partially done, host wiring deliberately deferred (2026-09-22)

- `roc_desk-common` gained `roc_desk_core::workspace` (tag `common-v0.5.0`)
  — the "open a local folder, remember it in a recent list" concept,
  **local-only** (the host's original also supports SSH/Agent remote
  workspaces, intentionally not replicated yet since that needs
  `roc_desk-ssh`'s connection pools, which this pass didn't wire together).
- `roc_desk-workspace` (tag `v0.1.0`) has: the workspace concept (via
  `roc_desk_core::workspace`), local terminal PTY, a local-only Git panel
  (status/diff/log/commit via `git` argv), all wired as commands and
  **actually smoke-tested through WebView2's CDP debug port** (opened a
  real git repo, verified `git_status`/`git_log` against ground truth,
  spawned a real `powershell.exe` PTY and streamed output) — this is the
  most thoroughly runtime-verified of any tool in this migration so far,
  including finding and fixing a real bug (missing `core:event:default`
  capability, which would have silently broken terminal output).
  Symbol indexing is not reimplemented here — it depends on
  `roc_desk-editor`'s `editor_symbols_*` (root-path-keyed, already a
  working precedent).
- **Explicitly NOT ported** (see the tool's own README/`lib.rs` module docs
  for the full reasoning): the AI coding agent loop (`coding::session`/
  `coding::tools`, ~3500 lines) and the file-change staging it drives
  (`coding::changes`/`diff`) — both depend on `crate::ai`/`crate::agent_llm`,
  which (same as the SQL Agent) has no `roc_desk_core` home yet. `skills`
  (Skills zip import) and `webfetch` were skipped for the same reason
  (undetermined `roc_desk-common` `common` package dependencies). The
  embeddable `<EditorPane/>` integration (plan §9/§10) was evaluated and
  deliberately skipped this pass — its own dependency tree (Monaco/pdfjs/
  mammoth/xlsx) plus cross-repo npm-via-git wiring was judged not worth the
  risk; the standalone shell has its own plain-textarea editor instead.
- **Host wiring NOT done — same blocker as SSH, and they're entangled**:
  host's `AppState.workspaces` (the open-workspace registry) is read by
  `commands/workspace.rs` (open/close — would be replaced),
  `commands/coding.rs` (15 sites — the AI Agent commands that stay in host,
  since Agent wasn't ported), and `commands/symbols.rs` (the workspace-keyed
  symbol commands, deliberately left in host, see the Editor section above).
  Swapping `roc_desk_core::workspace` in for `state.workspaces` without
  updating `coding.rs`/`symbols.rs` would split "workspaces open in the file
  tree" from "workspaces the AI Agent and symbol index know about" into two
  registries — the exact SQL-style regression, not attempted.
  **The correct fix** (future pass, likely combined with SSH's): once
  `roc_desk_core::ai`/`agent_llm` exist and the AI Agent is portable, migrate
  `coding.rs`'s Agent commands and `symbols.rs` together, at which point
  `state.workspaces` can be deleted from `AppState` entirely in favor of
  `roc_desk_core::workspace`'s registry, read by everyone.

## 深色/浅色主题一致性 + 一个重要的 npm 限制发现 (2026-09-23)

- 用户反馈两处真实 bug，都已修复并验证：
  1. `roc_desk-explorer` 独立 exe 完全没有主题切换按钮、`App.tsx` 全篇硬编码
     十六进制颜色（不是 CSS 变量），意味着这个工具事实上**只有一套写死的深色
     配色，没有真正的浅色主题**。补了 `themeStore.ts`/`ThemeToggle.tsx`（自
     包含，`data-theme` 属性 + `localStorage`，和宿主同一套模式），把
     `index.css` 补成真正的 `[data-theme="dark"]`/`[data-theme="light"]` 双
     主题令牌，`App.tsx` 的 `styles` 常量从字面量十六进制值换成 `var(--xxx)`
     引用。Tag `v0.2.3`。
  2. `roc_desk-editor` 独立 exe 界面外壳是深色的，但 Monaco 里打开的文件内容
     是浅色的——根因是 `CodeEditor.tsx` 请求名为 `roc-dark`/`roc-light` 的
     Monaco 自定义主题，但注册这两个主题的 `monacoSetup.ts` 从来没有搬进这个
     仓库，Monaco 找不到就静默回退到内置的浅色 `vs` 主题。把 `monacoSetup.ts`
     原样搬过来，在 `main.tsx`（独立壳入口）和 `index.ts`（给
     `roc_desk-workspace` 之类嵌入方用的库入口）两处都做一次性 side-effect
     import。同时给 `styles.css` 补上真正的双主题令牌（这个仓库组件历史上
     混用了 `--bg-app`/`--bg-base` 两套变量名，没有强行统一改名，而是让两套
     名字互为别名、都随主题切换）、加了顶部工具栏 + `ThemeToggle`。Tag
     `v0.2.3`。
  - 宿主 `Cargo.toml` 同步把这两个工具的 tag 提到 `v0.2.3`，`cargo check
    --release` 验证通过，跑了一次完整 `build-portable.ps1` 刷新
    `bin/roc_desk.exe`。

- **重要的架构发现：`docs/MULTI_REPO_SPLIT_PLAN.md` 里"npm 用
  `github:<repo>#path:packages/ui-core&<sha>` 取 monorepo 子目录"这个假设是
  错的，实测验证过**：
  ```
  npm install "github:swimhigh/roc_desk-common#path:packages/ui-core"
  npm error enoent Could not read package.json ...git-clone.../package.json
  ```
  plain npm 的 git 依赖只会克隆整个仓库、在**仓库根目录**找 `package.json`，
  不支持提取子目录（`path:` 片段不是 npm 认的语法，是我们自己在设计阶段的
  误记）。这意味着 `packages/ui-core`（本次顺手按计划搭建了 Toast/
  ContextMenu/ConfirmDialog/ThemeToggle/useFileTreeOperations 等组件，代码
  还在 `roc_desk-common` 仓库里、可以当参考实现抄）**目前没有一条实际可行
  的路径被其他仓库当 npm 依赖引用**——这正是为什么 `roc_desk-explorer`/
  `roc_desk-editor`/`roc_desk-ssh`/`roc_desk-workspace` 到目前为止全部是
  "各自拷一份 Toast/ContextMenu/ConfirmDialog 源码"而不是"依赖同一个包"：
  不是偷懒，是当前唯一可行的路。
  - 后续要真正做到"改一处、所有工具同步"，需要在下面几个方案里选一个（都
    没做，需要用户决策）：
    a) 把 `packages/ui-core` 单独拆成自己的 GitHub 仓库（根目录就是
       package.json），寄生在 `roc_desk-common` 之外——违背"9 个仓库"的
       设计，但是最省事、npm 原生支持。
    b) 发布到私有/公开 npm registry（`npm publish`），各工具正常
       `"@roc_desk/ui-core": "^0.2.0"` 依赖——需要维护发布流程和版本号。
    c) 换成 pnpm/yarn workspace + `workspace:` 协议——但这要求所有工具仓库
       和 `roc_desk-common` 在同一个 workspace 根下（即物理上合成一个
       monorepo），和"六个工具各自独立仓库"的设计冲突。
    d) 维持现状：`packages/ui-core` 只是"标准实现参考"，各工具手动同步拷贝
       （现在的实际做法），接受轻微的代码重复换取仓库独立性。
  - 在方案 a/b/c 选定之前，`packages/ui-core` 暂不建议投入更多组件迁移
    工作——写好的组件缺一条能被外部工具实际消费的路径，属于"写了但用不上"。

## AI 编程助手迁移 phase 1：共享 Agent 基础设施拆到 roc_desk_common (2026-09-24)

- **背景**：AI 编程助手（`coding::session`/`coding::tools`，约 3500 行）之前
  被判定为"单独一个大工程"，因为它依赖的 `crate::ai`/`crate::agent_llm`/
  `crate::coding::{CommandConfirmRegistry, QuestionRegistry, ChatAttachment}`
  这些基础设施完全是 host-only 的，哪个 tool repo 都拿不到。这不是假设——
  `roc_desk-sql` 早先那次迁移就因为同样的理由把 `sql/agent`（SQL Agent 的
  多轮工具调用循环）复制过去了但**没有在 `sql/mod.rs` 里声明**，成了一份
  写好但编译不到 crate 里的死代码。
- **这次做的**：把这些基础设施从 host 搬到 `roc_desk_common`（新增
  `common-v0.7.0`→`v0.9.0` 四个小版本），供任意 tool repo 共用：
  - `roc_desk_common::ai`：`AiProvider`/`AiProviderManager`/`AiProvidersRepo`
    （连同 SQLite 存储）、`AiChatClient`（流式对话+联网搜索）、`AiRuntime`
    （并发/取消控制）、`security`（脱敏/审计）、`sse`（SSE 解析）、
    `attachments`（图片/文本/PDF 附件处理，超预算时自动分窗口用 LLM 提取
    相关内容）。
  - `roc_desk_common::agent_llm`：协议无关的多轮工具调用引擎半层——
    chat/completions vs Responses API 两种协议归一化、429/5xx 重试、usage
    抽取。这是 `coding::session`/`sql::agent::session` 两边本来就在共用的
    同一份逻辑，只是之前只存在于 host 里。
  - `roc_desk_common::agent_confirm`：`CommandConfirmRegistry`/
    `QuestionRegistry`，两个通用的"等前端一个 oneshot 响应"注册表，和
    "文件/SQL"这些领域概念完全无关。
  - `roc_desk_common::agent_todo`：`TodoItem`/`TodoStatus`，`todo_write`
    工具用的结构化任务清单类型。
  - `event_prefix` 从硬编码的 `"coding:"` 改成参数（`"coding"`/
    `"sqlagent"`），这样两个 Agent 能共用同一份实现但各自发到自己的前端
    事件通道。
- **验证方式**：`roc_desk_common` 自己 `cargo test` 全绿（17 个测试，含从
  host 搬过来的 `ai::sse`/`ai::security` 单元测试）；然后把 `roc_desk-sql`
  那份死代码（`sql/agent/*.rs`、`sql/ai_assistant.rs`）的 import 从
  `crate::ai`/`crate::agent_llm`/`crate::coding::*` 改成
  `roc_desk_common::{ai, agent_llm, agent_confirm, agent_todo}`，在
  `sql/mod.rs` 里补上 `pub mod agent; pub mod ai_assistant;` 两行——
  `cargo check` 一次性编译通过，**证明这套共享基础设施确实可用**，不是
  纸面设计。目前只到"编译进 crate"这一步，还没有给 `roc_desk_sql::cmd`
  加对应的 Tauri 命令包装（standalone SQL 工具本身也还没有 Agent 面板的
  前端），这部分本来就不在这次的范围内。
- **NOT 做的、下一步真正的大工程**：把 `coding::session.rs`（2838 行）/
  `coding::tools.rs`（740 行）/`coding::changes.rs`/`diff.rs`/`git_ops.rs`/
  `guard.rs`/`permission.rs`/`skills.rs`/`webfetch.rs`（加起来约 4700 行）
  连同宿主侧命令层 `commands/coding.rs`（1532 行）迁到 `roc_desk-workspace`。
  这次没做，原因和之前文档记录的一样：`roc_desk_core::workspace::
  WorkspaceManager` 还是纯本地实现，没有宿主那套"已打开工作区注册表"
  （`state.workspaces`）和远程/SSH 工作区支持——`coding::session` 深度依赖
  这两者（每个 `CodingSession` 绑定一个 workspace，通过 `CodingTarget`
  区分本地/远程/Agent 目标读取对应的 `file_ops`/`ssh_pool`）。**这次拆出来
  的共享基础设施（ai/agent_llm/agent_confirm/agent_todo）是这个大工程的
  必要前置条件，但不是它本身**——真正开始搬 `coding::session.rs` 之前，
  还需要先解决 `WorkspaceManager` 的功能缺口（对齐或替换宿主实现），否则
  会重演"打开的工作区其实不可用"那类隐性回归。工作量比这次拆共享基础设施
  大得多，按这次的经验（一次带 8000 行左右代码、涉及 6+ 个仓库互相依赖）
  预计需要单独一次会有充足时间预算的迁移，不建议在时间紧张时强行推进。

## AI 编程助手迁移 phase 2：WorkspaceManager 补上远程工作区 + 本地打开注册表 (2026-09-24)

- **解决的正是 phase 1 结尾标出的那个前置条件缺口**：`roc_desk_core::
  workspace::WorkspaceManager` 之前是纯本地实现，没有宿主那套"已打开工作区
  注册表"（`AppState.workspaces`）和远程/SSH 支持，`coding::session` 深度
  依赖这两者，是搬它之前必须先解决的两个功能缺口。这次都做了：
  - **远程工作区（DB/profile 层）**：`common-v0.10.0` 给 `WorkspaceProfile`
    加回 `connection_id`/`last_sftp_local_path`/`last_sftp_remote_path`
    字段和 `WorkspaceKind::Remote`，`WorkspaceManager` 加了
    `open_remote`/`update_last_sftp_paths`。**这个 crate 依然不依赖
    `roc_desk-ssh`**——`open_remote` 不自己解析 `connection_id`，接受调用方
    已经算好的 `display_name`/`embedded_workspace_id`（调用方才有
    `roc_desk-ssh` 的连接池，能探测远程主机上的 `.rock_desk/workspace.json`
    标记文件），这里只管 DB upsert + 本机 fallback 缓存那一半，职责边界和
    `roc_desk_common`（不依赖桌面宿主/具体工具）的定位一致。
  - **本地打开工作区注册表**：`roc_desk-workspace@v0.2.6` 给
    `WorkspaceAppState` 加了 `open_workspaces: Arc<RwLock<HashMap<Uuid,
    WorkspaceHandle>>>`，`workspace_open_local` 现在会往里插一条（之前
    发现的真实 gap：它原来什么都不插，和宿主的 `workspace_open_local` 行为
    不一致），新增 `workspace_close` 命令负责移除。`WorkspaceHandle` 里的
    `file_ops: Arc<dyn roc_desk_common::fsops::FileOps>` 目前只接
    `LocalFileOps`——远程工作区要接进来，还需要这个 crate 直接依赖
    `roc_desk-ssh`（获取 `ConnectionManager`/`SshConnectionPool`/
    `AgentConnectionPool`），这是特意留到下一步的，不在这次范围内（见下）。
- **验证方式**：`roc_desk_core`/`roc_desk_common` 全部单测通过（17 个，
  含新增字段不影响任何既有测试）；6 个 tool repo + 宿主重新对齐到
  `common-v0.10.0`（这次的版本漂移修复流程比之前更熟练——`roc_desk-explorer`
  → `roc_desk-editor`（依赖 explorer）→ 剩下互相独立的 4 个仓库，逐个
  `cargo check` 验证单一 `roc_desk_core` 实例后再提交打 tag，没有再重演
  "本地改完 Cargo.toml 但忘记给依赖它的仓库也发新 tag"这个坑）；宿主
  `cargo check` 全绿，`build-portable.ps1` 完整跑通。
- **仍然没做、下一步真正要做的事**（按依赖顺序）：
  1. 给 `roc_desk-workspace` 加 `roc_desk-ssh` 依赖，让它自己能解析
     `connection_id` → 实际连接 → `RemoteFileOps`/`AgentFileOps`，补上
     `workspace_open_remote` 命令，把 `WorkspaceHandle.file_ops` 扩成
     "本地或远程都行"。这一步之后，`open_workspaces` 注册表才是真正完整的
     "宿主 `AppState.workspaces` 的对等物"。
  2. 决定"谁的 `ConnectionManager`/`SshConnectionPool`/`AgentConnectionPool`
     实例"这个问题——如果宿主接线（`roc_desk-workspace` 的 `open_remote`
     命令最终要接进宿主），必须复用宿主已经在用的 `RocDeskSshAppState`
     那一份，不能自己重新 `new` 一份，否则重演这次会话反复踩过的"分裂
     注册表"bug（同一个连接池在两个地方各有一份，互相看不到）。standalone
     的 `roc_desk-workspace` 单独跑的场景要不要也支持远程工作区（意味着
     要嵌入一部分 SSH 连接管理 UI/命令）是一个单独的产品决定，不是这次
     范围。
  3. 真正开始把 `coding::session.rs`（2838 行）/`coding::tools.rs`
     （740 行）/`coding::changes.rs`/`diff.rs`/`git_ops.rs`/`guard.rs`/
     `permission.rs`/`skills.rs`/`webfetch.rs`（加起来约 4700 行）连同宿主
     命令层 `commands/coding.rs`（1532 行）迁到 `roc_desk-workspace`——现在
     前置条件（共享 Agent 基础设施 + WorkspaceManager 远程支持 + 打开注册表）
     都齐了，但这一步本身仍然是这次会话里最大的单项工作量，需要单独一次
     会话专门做。

## AI 编程助手迁移 phase 3：`roc_desk-workspace` 真正接上远程工作区 (2026-09-24)

- 完成了 phase 2 结尾列的第 1、2 项：`roc_desk-workspace@v0.2.7` 加了
  `roc_desk-ssh` 依赖，新增 `WorkspaceAppState::with_ssh(connection_manager,
  ssh_pool, agent_pool)` —— **这是个 opt-in builder，不是构造时必填**，
  理由直接写在新增的 `SshPools` 类型文档里：连接池该由谁构造/持有是调用方
  的决定，这个 crate 不能替调用方做主。宿主接线时必须传进去宿主自己
  `RocDeskSshAppState` 的那几个 `Arc`，不能在这里 `new` 一份新的——否则
  正是这次会话反复踩过的"分裂注册表"（SSH 面板里连的连接，这边看不见）。
  standalone 没有自己的连接管理 UI，`ssh` 字段保持 `None`，`workspace_
  open_remote` 这时候会返回"远程工作区功能未启用"的明确错误，不是 panic
  或者静默失败。
- `cmd::workspace_open_remote` 镜像宿主旧版 `commands::workspace::
  workspace_open_remote` 的行为：探测远程 `.rock_desk/workspace.json`
  标记文件决定要不要复用已有 id，打开成功后把 `WorkspaceHandle`（带
  `RemoteFileOps`/`AgentFileOps`）插进 `open_workspaces` 注册表，元数据
  文件写回远程走 best-effort（只读目录不应该让"打开只读工作区"这个操作
  本身失败）。
- 顺手发现并修了一个遗漏：`workspace_close`/`workspace_update_last_sftp_
  paths` 两个命令在 v0.2.6 就加了，但从来没有在 `standalone/src/main.rs`
  的 `generate_handler!` 里注册过——这次一并补上。
- **宿主侧还是没接**：宿主 `commands/workspace.rs` 自己的
  `workspace_open_local/open_remote/...` 10 个命令依然原样保留，没有切换
  成 `roc_desk_workspace::cmd::*`——这是 phase 2 就记录过的、故意的决定：
  宿主自己的 `WorkspaceManager`/`state.workspaces` 目前还是功能超集
  （`commands/coding.rs` 15+ 处、`commands/symbols.rs` 都依赖它），贸然切换
  会导致"AI 助手/符号索引看到的工作区"和"文件树看到的工作区"分裂成两个
  注册表。`roc_desk-workspace` 现在已经具备和宿主对等的能力（本地+远程打开、
  注册表、SFTP 记忆路径），但"宿主改用这个 crate 代替自己的实现"要和
  `coding::session.rs` 真正迁移那一步一起做，不是提前单独切，否则中途会有
  一段两套工作区概念并存、容易出 bug 的过渡态。
- **验证方式**：`roc_desk-workspace` 的 `lib`/`standalone` 都 `cargo check`
  通过，`Cargo.lock` 确认 `roc_desk_core` 单一实例；宿主 bump 到
  `roc_desk-workspace@v0.2.7` 后 `cargo check` 全绿（宿主目前不调用新加的
  `workspace_open_remote`，纯粹是依赖图更新，功能行为不变）。
- **到这里，phase 2 列的三件事已经完成两件半**（远程 DB/profile 支持 +
  本地/远程打开注册表都有了，连接池归属问题也有了明确、不会分裂注册表的
  设计），真正剩下的就是 phase 2 第 3 项：把 `coding::session.rs` 本身
  （连同 `coding::tools/changes/diff/git_ops/guard/permission/skills/
  webfetch` 和宿主 `commands/coding.rs`，加起来约 6200 行）迁进来，这仍然
  是单独一次会话量级的工作，不建议现在零散时间里强推。

## 环境问题记录：本机杀毒软件间歇性拦截刚编译出的 Rust 构建脚本 (2026-09-24)

- 本次会话反复撞到 `error: failed to run custom build command for
  \`<crate>\`... 拒绝访问 (os error 5)`——一开始怀疑磁盘空间不足（`F:` 盘
  一度只剩 42MB，清理 `roc_tools/*/target` + `roc_desk/build` 后恢复到
  40GB+，但清完之后同类错误仍然出现），后确认是 360 安全卫士
  （`360tray.exe`/`360Safe.exe`）对刚编译出来、还没签名的 Rust 构建脚本
  可执行文件做实时扫描，扫描窗口内 cargo 想执行/重命名这个文件会拿到
  `ACCESS_DENIED`。规律：换一个新的 crate 名字第一次触发时几乎总是失败，
  但同一个 crate 的构建脚本一旦在某个仓库的 `target/` 里成功跑过一次，
  它自己的 `target/` 目录里就不会再触发（不是全局免疫，是"这个具体文件在
  这个具体路径第一次执行"这一步容易撞上扫描窗口）——单纯重试（不改任何
  代码）５～２０次基本都能过去，最长一次卡了 40 次重试才通过（`tauri-
  runtime`/`tauri-runtime-wry`/`encoding_rs` 是这次撞到最多次的几个）。
  跨仓库复制已经编译好的 `build-script-build.exe` 大多数时候没用——cargo
  的 fingerprint 校验会认为"不是自己刚编译的"而重新编译一遍，重新触发同一
  个扫描窗口。**结论：没有真正的修复手段，只能重试**；如果这个问题反复出现
  拖慢构建，需要用户自己在 360 里给 `F:\code\wuyou` 加一条实时防护排除规则。
- 顺带确认（并修复）了一个真实的、和这个环境问题无关的 bug：`crates.io`
  的 `index.crates.io`/`static.crates.io` 在本机网络下 TLS SNI 校验持续
  失败（GitHub 连接正常，只有 crates.io 系域名不通），已经给用户全局
  `~/.cargo/config.toml`（不只是 `roc_desk` 项目自己的 `.cargo/
  config.toml`）加了中国科技大学的 crates.io 镜像源，所有仓库的构建都受益。

## Known cross-tool dependency-graph quirk — recurred twice, still needs a real fix (2026-09-24)

- This version-drift bug (multiple repos pinning different `common-v*`
  tags → cargo resolving two incompatible `roc_desk_core` instances) has now
  been hit and manually fixed **twice**: once earlier this session (aligned
  everyone on `common-v0.5.0`), and again today — bumping `roc_desk-common`
  to `common-v0.6.0` (added `paths::portable_data_dir`) and updating the six
  tool repos' own `roc_desk_core`/`roc_desk_common` tags missed two
  *transitive* pins: `roc_desk-editor`'s own dependency on `roc_desk_explorer`
  (still `v0.2.4`, itself still pinning `common-v0.5.0`) and
  `roc_desk-workspace`'s dependencies on both `roc_desk_editor` and
  `roc_desk_explorer` (same stale `v0.2.4`). `cargo check` did **not** error
  on this — it silently compiled two `roc_desk_core` instances side by side,
  same as the first time this happened; it only becomes a hard compile error
  if some code actually passes a value across the two instantiations'
  boundary. Both are fixed now (`roc_desk-editor@v0.2.6`,
  `roc_desk-workspace@v0.2.4`, everyone on `common-v0.6.0`), but the *process*
  gap is unfixed: there's no automated check that catches this, it's manual
  vigilance every time `roc_desk-common` bumps. A `cargo tree -d` (duplicates)
  check in each tool repo's CI, or a script that greps every repo's
  `Cargo.toml` files for `common-v` tags and fails on disagreement, would
  catch this mechanically instead of relying on someone noticing weird
  double-compilation in build output. Not implemented yet.

## Standalone tool packaging: portable data dir + release bundling (2026-09-24)

- **Bug**: every standalone tool exe (`roc_desk-ssh`/`sql`/`workspace`/`http`
  standalone; `editor`/`explorer` have no persistent state so were unaffected)
  resolved its SQLite data directory via Tauri's OS AppData default (or, for
  `roc_desk-workspace`, a bespoke `dirs_next_data_dir()` pointing at
  `%APPDATA%/roc_desk-workspace`) — a different location per tool, and
  different from the full `roc_desk.exe` host's own convention (exe-relative
  `.rock_desk/`, portable-zip-friendly). Running two standalone tools side by
  side, or a standalone tool alongside the full host, meant each kept its own
  disconnected data — the same "split registry" class of bug this migration
  has repeatedly had to design around, just at the packaging layer instead of
  in-process wiring (2026-09-24 user feedback).
- **Fix**: added `roc_desk_core::paths::portable_data_dir()` (new in
  `common-v0.6.0`) — same exe-relative `.rock_desk` resolution the host uses,
  minus the legacy-directory-migration logic (standalone tools are new, no
  legacy dir to migrate from). Updated `roc_desk-ssh`/`sql`/`workspace`/`http`
  standalone `main.rs` to use it. Consequence: copying several standalone
  tool exes into the *same* directory now makes them automatically share one
  `.rock_desk` (each tool still uses its own filename inside it, e.g.
  `roc_desk_ssh.db`/`workspace.db`, so no collision).
- **Also found while doing this**: `roc_desk-ssh`'s standalone build never
  bundled `wfreerdp.exe` (the RDP-embedding runtime dependency, only ever
  vendored in the host repo's own `vendor/`) — running the standalone SSH
  tool's RDP feature alone would fail for lack of it. Vendored a copy into
  `roc_desk-ssh/vendor/` and updated `build-standalone.ps1`/CI to bundle it
  alongside the exe.
- **Process mistake made and fixed along the way**: accidentally used
  `git add -A` while committing the `roc_desk-ssh` fix, which swept in ~150
  files of unrelated pre-existing uncommitted work in that repo's working
  tree (frontend component refactor, `standalone/dist/` build output,
  `package-lock.json`). Cleaned up in a follow-up commit; the real frontend
  source changes were kept (they were genuine in-progress work), only the
  `standalone/dist/` build artifacts were removed and gitignored. This
  surfaced a second, real bug: **`standalone/dist/` had been committed
  straight into git in all six tool repos because `build-standalone.ps1`
  never actually ran the frontend build** (`vite`'s `outDir` writes directly
  to `standalone/dist`, but nothing invoked `npm run build` — `tauri.conf.json`
  has no `beforeBuildCommand`). Gitignoring `dist/` without fixing this would
  have permanently broken local builds and CI. Fixed by adding an explicit
  `npm install && npm run build` step to every repo's `build-standalone.ps1`
  and a `setup-node` step to every repo's CI workflow, verified by an actual
  clean local build of all six.
- **`roc_desk-releases` bundling**: added a `workflow_dispatch` GitHub Actions
  workflow (`.github/workflows/bundle.yml`) that pulls the latest published
  release asset from each of the six tool repos (plus `wfreerdp.exe`) and
  republishes them together as one zip — extracting it puts every tool in
  one folder, which is what makes the `portable_data_dir` sharing above
  actually happen for an end user. **Not yet triggered/verified** — creating
  and running a GitHub Actions workflow isn't observable from this
  environment; needs a manual `workflow_dispatch` run to confirm it actually
  works end-to-end (gh CLI auth, `--pattern` matching, zip contents).

## AI 编程助手迁移 phase 4：`coding::session.rs` 的全部支撑模块迁移完成 (2026-10-08)

Continuing the AI coding agent migration from phase 1-3 above: `roc_desk-workspace`
now has every module `coding::session.rs` (host, 2838 lines — the actual
multi-turn tool-calling loop, not yet itself ported) depends on, each ported
module-by-module with its own `cargo check` + `cargo test` + tag + push
before moving to the next, same discipline as the earlier phases:

- `coding::target` (`CodingTarget`), `coding::local_exec` (local shell exec,
  Windows PowerShell/cmd.exe dispatch), `coding::diff`, `coding::guard`
  (command blacklist/whitelist), `coding::permission` (`PermissionEngine` +
  `PermissionRulesRepo` folded in with its own `ensure_schema()`),
  `coding::git_ops`, `coding::changes` (`ChangeStore`/`FileChange`),
  `coding::tools` (`ToolCall` enum, `tool_schema()`, `apply_text_edit`) —
  `v0.2.7` through `v0.2.10`, done in an earlier session and already
  recorded by the files-and-code-sections tracking at the time.
- `coding::webfetch` (`fetch_url` + SSRF guard against internal/loopback
  hosts) — `v0.2.11`.
- `coding::skills` (`SKILL.md` discovery/frontmatter parsing, `.zip`/
  `.tar.gz`/`.tgz` import with the same "找唯一含 SKILL.md 的子目录" probing
  as the host) — `v0.2.12`.
- `coding::audit` (`AuditLogRepo`, `spawn_blocking`-backed command audit
  log) and `coding::evidence` (`AiEvidenceRepo` — the FTS5-backed cache of
  file-snapshot/search/webfetch results the agent has already gathered,
  letting a later turn recall a prior finding instead of re-fetching it) —
  `v0.2.13`. Added two new tests exercising the FTS5 virtual table end to
  end (`ensure_schema` → `upsert` → `search_fts`) specifically because FTS5
  support in a `rusqlite`/`libsqlite3-sys` "bundled" build isn't guaranteed
  by the Cargo feature alone — confirmed working rather than assumed.
- `coding::mcp` (`McpServerManager`, stdio child-process transport, HTTP
  "Streamable HTTP" transport with the minimal SSE-frame parser real MCP
  servers need, `McpServersRepo`) — `v0.2.14`.
- Also added to `roc_desk_common` itself (not `roc_desk-workspace`):
  `fsops::search` (`search_stream`/`SearchOptions`/`SearchMode`, the
  directory-walking content/filename search `session.rs`'s `search_files`
  tool uses) — `common-v0.11.0`, since this is generically useful to any
  tool crate with a `FileOps` tree, not coding-agent-specific.

**What's left**: `coding::session.rs` itself — the `CodingSession` struct,
system-prompt construction, the `send_message` tool-calling loop (context-
window budgeting/summarization, MCP tool dispatch, background job
management, the `execute_tool` match over all ~24 `ToolCall` variants
including `task` sub-agent recursion, `run_command_gated`/`webfetch_gated`
permission+confirmation flows), plus host's `commands/coding.rs` (1532
lines, the Tauri command layer calling into it). This is the single
largest remaining unit of work and is deliberately **not** attempted in one
pass — it's dense, security-relevant (command execution gating, permission
rules), and threads together every module ported above plus the `ai`/
`agent_llm`/`agent_confirm`/`agent_todo` infra from phase 1. Next session
should continue reading `session.rs` from where phase 4 left off (its
`execute_tool` match, `run_command_gated`, `run_subagent_task`, background
job commands) and port it incrementally with the same per-chunk
check/test/commit discipline, rather than writing the whole thing in one
untested pass.

## AI 编程助手迁移 phase 5：`coding::session.rs`（2838 行）本体迁移完成 (2026-10-08)

Finished what phase 4 left as "what's left" — `coding::session.rs` itself is
now fully ported into `roc_desk-workspace/lib/src/coding/session.rs`
(`v0.2.15`), and `roc_desk-workspace` can now, in principle, run the whole
AI coding agent end to end (not yet wired into any Tauri command or
standalone UI — that's the next layer, see below).

- Read the entire 2838-line file end to end and confirmed every host-only
  dependency it has was already either ported in an earlier phase, or
  already exists byte-for-byte in `roc_desk_common` from phase 1
  (`build_user_message_content`/`ChatAttachment`/`condense_attachment_text`/
  `extract_pdf_text_raw` turned out to already live verbatim in
  `roc_desk_common::ai::attachments`, just parameterized by `event_prefix`
  — host's own `session.rs` simply never switched over to the shared
  version after phase 1 extracted it for the SQL Agent's benefit, so this
  phase could delete ~280 lines of would-be duplication and call the
  common version directly instead of re-porting it) — before writing a
  single line, not discovered partway through.
- No new infrastructure-layer dependency turned out to be missing: every
  remaining dependency `session.rs` has (`fsops::search_stream`,
  `symbols::build_index`/`SymbolIndex`, `agent_llm::*`,
  `agent_confirm::{CommandConfirmRegistry, QuestionRegistry}`,
  `AuditLogRepo`/`AiEvidenceRepo`/`McpServerManager`) was already in place
  from phases 1-4, confirmed by grepping every `use crate::`/`use super::`
  line in the host file before starting rather than discovering gaps
  mid-port. This phase's actual work was entirely in `session.rs` itself.
- `CodingTarget`/`ChangeStatus`/`FileChange`/`FileSyncInfo` are *not*
  redefined in the ported `session.rs` (host defines them inline at the top
  of its own `session.rs`) — the tool-repo versions already live in
  `coding::target`/`coding::changes` from phase 4's porting, so the ported
  `session.rs` imports them from there instead of duplicating the type
  definitions, a deliberate structural difference from the host file.
- `effective_context_budget`/`DEFAULT_CONTEXT_TOKENS_ESTIMATE`/
  `CONTEXT_BUDGET_HEADROOM_{NUM,DEN}` are also dropped from the port —
  confirmed these are byte-for-byte the same formula as
  `agent_llm::context_budget` (already shared from phase 1), so the port
  calls that directly instead of keeping a second copy of the same 60_000-
  token-default calculation.
- Pulled in two new direct dependencies: `roc_desk_protocol` (same repo/tag
  as `roc_desk_ssh`, needed for the `Agent`-target branch of
  `search_files_uncached`, which speaks the Agent wire protocol directly)
  and `tokio-util`/`sha2`/`chrono`/`tracing`/`async-trait` (accumulated
  across phase 4 and 5 for `CancellationToken`, evidence-cache hashing,
  timestamps, warn-logging, and the MCP transport trait respectively).
  `windows_command_for`/`unix_command_for` in `coding::local_exec` had to be
  changed from `fn` to `pub(crate) fn` — phase 4 ported them without
  anticipating `session.rs`'s `run_command_background_gated` would need to
  call them directly from a sibling module.
- **Triggered (and fixed) the recurring cross-repo version-drift bug one
  more time**: bumping `roc_desk-common` to pick up phase 4's
  `fsops::search` addition (`common-v0.11.0`) meant `roc_desk-workspace`'s
  other git dependencies (`roc_desk-editor`/`roc_desk-explorer`/
  `roc_desk-ssh`, each still pinned to `common-v0.10.0`) would otherwise
  resolve a second, incompatible `roc_desk_core`/`roc_desk_common` instance.
  Fixed in the established dependency order — `roc_desk-explorer`
  (`v0.2.9`) → `roc_desk-editor` (`v0.2.9`, depends on explorer) →
  `roc_desk-ssh` (`v0.3.6`, independent) — each bumped, `cargo check`'d
  alone to confirm a single resolved instance, committed/tagged/pushed,
  *before* bumping `roc_desk-workspace`'s own references to all four and
  verifying its build one final time. Host's own `Cargo.toml` is
  deliberately **not** bumped to any of these new tags — it doesn't yet
  consume anything from this phase's work, and bumping it would just be
  unnecessary churn ahead of actual need (same rule phase 1-4 followed).

**What's left**: host's `commands/coding.rs` (1532 lines, the Tauri command
layer — `coding_start`/`coding_send_message`/accept/reject/undo/redo/
history-save/-resume, the `build_new_session` constructor that wires a
freshly-built `CodingSession` together with `tokio::join!`-concurrent
project-memory/skill/git-repo probes) has not been looked at yet in this
migration. Until that layer is ported and exposed as Tauri commands (plus
a `WorkspaceAppState`-level registry of active `CodingSession`s, analogous
to host's `AppState.coding_sessions`/`coding_changes`/
`coding_pending_injections`/`coding_cancel_tokens`), `roc_desk-workspace`'s
AI coding agent is a fully-compiling, fully-tested library with no command
surface calling into it yet — the next session should read
`commands/coding.rs` in full before starting, the same way phase 5 started
by reading all of `session.rs` first, since the command layer is where the
per-workspace session registry and concurrency/locking strategy actually
get decided, not something to design file-by-file as it's ported.

## AI 编程助手迁移 phase 6：`commands/coding.rs` 命令层迁移完成，tool-repo side done (2026-10-08)

What phase 5 left as "what's left" is now done: host's `commands/coding.rs`
(1532 lines) is fully ported into `roc_desk-workspace` (`v0.3.0`) as
`coding::commands` (pure helper logic — caches, `build_new_session`,
`maybe_auto_continue`, history list/resume reconciliation) plus ~30
`#[tauri::command]` wrappers added to `cmd` in `lib.rs`. `roc_desk-workspace`
now has a complete, wired, standalone-buildable AI coding agent — not just a
compiling library with no caller, as phase 5 left it.

- **New `coding::history` module** (`CodingHistoryRepo`/`CodingHistoryInput`/
  `CodingHistorySummary`/`CodingHistoryDetail`/`WorkspaceHistorySnapshot`) --
  host's `db::repo::coding_history_repo.rs` wasn't covered by any earlier
  phase (it's a DB repo, not something `session.rs` itself imports), found
  only once `commands/coding.rs` was read end to end. Same
  `ensure_schema()`-folded-into-the-repo pattern as `permission`/`audit`/
  `evidence`/`mcp::repo` from phases 4-5.
- **`WorkspaceAppState` grew the full set of fields `CodingSession`/
  `commands/coding.rs` need**: `coding_sessions`/`coding_changes` (session +
  change-store registries, independently locked per the host's own
  rationale), `coding_history`/`ai_evidence`/`audit_log`/`permission_rules`/
  `mcp_manager` (all sharing this tool's one SQLite file via a cloned
  `DbPool`, each calling its own `ensure_schema()` at construction —
  previously these `ensure_schema()` methods existed from phases 4-5 but
  were never actually *called* by anything), `ai_provider_manager` (new:
  constructs `roc_desk_common::ai::{AiProvidersRepo, AiProviderManager}`
  with a `roc_desk_core::credential::KeyringStore`, this tool's first use of
  either), `command_confirms`/`question_confirms`/`coding_cancel_tokens`/
  `coding_pending_injections`/`symbol_indexes`.
- **`WorkspaceHandle` gained a `fallback_cache_dir` field** (host's
  `workspace::WorkspaceHandle` has the equivalent) -- needed by
  `coding_history_save`'s best-effort local mirror when a remote workspace
  write fails. Required exposing a new `WorkspaceManager::cache_root()`
  accessor from `roc_desk_core` (previously private) rather than duplicating
  the cache-root path computation independently.
- **New `fsops::copy_between` landed in `roc_desk_common` itself**
  (`common-v0.12.0`, bundled with the `cache_root()` accessor as
  `common-v0.11.1`/`v0.12.0`): cross-`FileOps`-implementation file/directory
  copy, needed by `skill_import` to copy a local skill folder into a
  possibly-remote workspace -- generically useful beyond the coding agent,
  so it went into `roc_desk_common::fsops` directly rather than being
  duplicated inside this tool crate, mirroring the earlier decision to put
  `fsops::search_stream` there instead of in `roc_desk-workspace`.
- **A design decision, not a bug**: every coding-agent command now requires
  `WorkspaceAppState::with_ssh` to have been called, *even for a session
  whose target ends up local* — `CodingSession::send_message`'s signature
  unconditionally takes concrete `SshConnectionPool`/`AgentConnectionPool`
  references (unused on the `Local` path, but still part of the call), and
  this crate has no way to construct meaningful placeholder pools of its
  own. This matches the host's own precedent exactly: host's
  `coding_send_message` always takes `State<'_, RocDeskSshAppState>`
  regardless of the session's actual target, because the host process
  always has that state managed. The standalone build (which never calls
  `with_ssh`) registers every `coding_*`/`mcp_server_*`/`permission_rule_*`/
  `skill_*` command anyway for forward compatibility, same policy already
  established for `workspace_open_remote` -- they return a clear "AI 编程
  助手功能未启用" error at runtime rather than being omitted.
- **Triggered the version-drift bug a third time, caught earlier than
  before**: after bumping `roc_desk-explorer`/`roc_desk-editor`/`roc_desk-ssh`
  and `roc_desk-workspace/lib`'s own references to `common-v0.12.0`, a
  `cargo check` on `roc_desk-workspace/standalone` still showed **two**
  `roc_desk_core` instances being compiled (`common-v0.12.0` *and*
  `common-v0.10.0`) -- `standalone/Cargo.toml` has its *own* direct
  `roc_desk_core` dependency (for `paths::portable_data_dir()`) separate
  from `lib`'s, which this round's bump had missed. Caught by actually
  running `cargo check` on the standalone crate specifically (not just
  `lib`) before considering the bump done, then confirmed fixed via `grep
  -c 'name = "roc_desk_core"' Cargo.lock` showing exactly one entry in the
  workspace-level lockfile. Lesson for next time: **every crate in a
  multi-crate repo with its own direct git dependency on `roc_desk-common`
  needs checking individually** — `lib`'s own `cargo check` passing is not
  sufficient proof the whole repo is drift-free when `standalone`/other
  members pin the same dependency separately.

**What's left**: host itself still runs its own original, never-touched
`coding::session`/`commands/coding.rs` — nothing in the host repo has been
changed to *consume* `roc_desk-workspace`'s now-complete AI coding agent.
"Host wiring" (replacing host's own implementation with calls into
`roc_desk_workspace::cmd::coding_*`, the same transition already done for
SSH/SFTP/RDP/Agent and partially done for the workspace/PTY/Git-panel
pieces earlier in this migration) is a distinct, deliberately-deferred next
phase — see the "SQL 工作台（tool-repo side）"/"SSH/SFTP/RDP/Agent（tool-repo
side）" sections above for the established pattern of what that transition
looks like when it's done. `roc_desk-workspace`'s own standalone build is
untested end-to-end in a running app (verified via `cargo check`/`cargo
test` only, consistent with every other phase in this migration) — running
it has not been attempted in this environment.

**2026-10-08 follow-up — host wiring found to be a bigger decision than
"paste and register", explicitly deferred by the user**: attempting host
wiring surfaced a real architectural fork, not a mechanical step. Host's
existing `roc_desk_workspace::WorkspaceAppState` instance (added earlier in
this migration for the PTY/Git-panel-only wiring) is a *second, independent*
workspace registry — it points at its own separate `workspace_tool.db` and
its `open_workspaces` map is never populated by host's actual
"open workspace" commands, which still go through `AppState.workspaces`/
host's own fuller `WorkspaceManager` (see the host code comment at the
`WorkspaceAppState::new` call site: "workspace_open/list 等命令仍然用宿主
自己更完整的 WorkspaceManager"). But every coding-agent command this phase
added resolves its `WorkspaceHandle` via `WorkspaceAppState.open_workspaces`
— wiring them into host as-is would mean every `coding_start`/
`coding_send_message` call looks up a workspace in a table host never
writes to, and fails outright.

Three ways to resolve this were presented to the user:
1. Defer host wiring entirely for now (matches the already-established
   SQL/SSH precedent).
2. Make `roc_desk_workspace`'s registry canonical in host too — redirect
   host's own `workspace_open_local`/`workspace_open_remote` commands to
   `roc_desk_workspace::cmd::workspace_open_*`, retiring `AppState.workspaces`/
   the host's standalone `WorkspaceManager` instance. Correct long-term
   direction, but a materially larger change (every piece of host code that
   reads `AppState.workspaces` would need to move too), not something to
   fold into a "wire up the coding agent" task.
3. Keep both registries, with host's workspace-open commands additionally
   mirroring into `workspace_app_state.open_workspaces` — smaller, lower-
   risk, but a patch that keeps two sources of truth in sync rather than a
   real fix.

**User chose option 1** — host wiring for the AI coding agent stays
deferred, same as SQL/SSH. Noted here specifically (rather than just
repeating the same one-line "deferred" the other tools got) because the
*reason* is different: those were deferred for scope/time reasons; this one
surfaced an actual pre-existing architectural inconsistency in the host
(two independent workspace registries) that option 2 above would need to
resolve before host wiring could even start, and that's a decision for a
dedicated future task, not something to default into while wiring one
feature.

## AI 编程助手迁移 phase 7：`roc_desk-workspace` 自己的前端界面 (2026-10-08)

With host wiring deliberately deferred, the user asked to instead give
`roc_desk-workspace`'s own standalone build a working AI coding agent
*frontend* — until this phase, the tool-repo side was backend-only
(commands existed, nothing in `src-web` called them). Ported from host's
`src-web/src/components/CodingAgent/*`, `stores/codingStore.ts`, and the
provider-management half of `stores/aiChatStore.ts`/`ProviderManagerDialog.tsx`
(~3300 lines across both repos combined) — `v0.3.1`-`v0.3.2`.

- **Styling is deliberately not a pixel match of the host.** The user chose
  this explicitly after being told host's AI-agent styling lives inside one
  1800-line `components.css` shared by every panel in the app, with no
  clean way to extract just the coding-agent-relevant classes. Instead
  wrote a compact, functional set of classes in this tool's own
  `styles.css` (~230 new lines) reusing the design tokens
  (`--bg-surface`/`--border-default`/`--accent`/etc.) and the `.btn`/
  `.dialog-*` patterns this repo already had from the terminal/Git-panel
  work. Visually simpler than host, not broken.
- **New backend commands needed and added**: `ai_provider_list/create/
  update/delete/list_models` -- these live in the host's separate
  `commands/ai.rs`, not `commands/coding.rs`, so phase 6's port correctly
  didn't bring them along, but the frontend has no usable coding session
  without at least one configured provider. Thin wrappers around the
  `ai_provider_manager` field phase 6 already added to `WorkspaceAppState`
  (`v0.3.1`).
  - Also added `roc_desk_common::fsops::copy_between` (needed by
    `skill_import`)'s companion piece on the frontend side needed no new
    backend work -- the backend half was already done in phase 6.
- **`aiProviderStore.ts` is a new, trimmed store**, not a straight port of
  host's `aiChatStore.ts` -- the host version also manages a general-
  purpose streaming chat panel (`ai_chat_send`/`ai:chat-chunk` events) this
  tool doesn't have and wasn't asked to add. Only the provider CRUD/model-
  list half was ported; `codingStore.ts`'s `saveCurrentHistory` and
  `CodingAgentPanel.tsx`'s provider/model pickers read from this instead.
- **A few small, previously-unported host utilities turned out to be load-
  bearing and got ported too**: `utils/markdown.ts` + `utils/shellHighlight.ts`
  (Markdown rendering and shell-command syntax coloring inside the
  timeline, pulled in `marked`+`dompurify` as new frontend deps),
  `utils/language.ts` (Monaco language-id detection for the file-change
  Diff viewer), `utils/formatTokens.ts`, `hooks/useExternalFileDrop.ts`
  (Tauri's window-level native drag-and-drop, needed because
  `dragDropEnabled` defeats plain HTML5 `ondrop` on Windows), and two tiny
  shared components (`SegmentedControl`, `ToggleSwitch`).
- **Verification**: both `tsc --noEmit` (strict mode, `noUnusedLocals`/
  `noUnusedParameters` on, checks every file under `src/` regardless of
  whether anything imports it) and a full `vite build` pass cleanly with
  zero errors/warnings beyond vite's pre-existing "Monaco chunk is large"
  notice. This is **not** the same as having run the app and clicked
  through it in a browser/webview — no such verification was attempted in
  this environment; it confirms the code is well-typed and bundles, not
  that the UI behaves correctly at runtime.
- **A real, not-yet-resolved usability gap this phase surfaced**: every
  `coding_*` command phase 6 added requires `WorkspaceAppState::with_ssh`
  to have been called (see that field's doc comment -- `CodingSession::
  send_message`'s signature unconditionally needs concrete `SshConnectionPool`/
  `AgentConnectionPool` references even though a `CodingTarget::Local`
  session never touches them). `roc_desk-workspace`'s own `standalone/
  src/main.rs` **never calls `with_ssh`** -- it has no SSH connection-
  management UI of its own. Concretely: a user running the standalone
  `roc_desk-workspace.exe` and opening a **local** folder will see the new
  "AI 编程助手" tab, but `coding_start` will immediately fail with "AI 编程
  助手功能未启用" even though their session never needed SSH at all. This
  wasn't something the user explicitly signed off on (it's a side effect of
  phase 6's design, not surfaced clearly as "this breaks the common
  standalone+local case" at the time). Not fixed in this phase -- doing so
  would mean either constructing throwaway `SshConnectionPool`/
  `AgentConnectionPool` instances for the local-only case (pulls in SSH
  connection-management machinery the standalone build has no other use
  for) or restructuring `CodingSession::send_message`'s signature to make
  the pools optional. Flagging this explicitly rather than letting it be
  discovered as a confusing runtime error later.

## 独立版对齐 roc_desk.exe 的三阶段规划 (2026-10-08)

User goal, stated directly: `roc_desk-workspace.exe`/`roc_desk-sql.exe`
standalone should have **full feature parity** with the corresponding
module in `roc_desk.exe` -- not just "compiles and runs", but local *and*
remote workspaces, AI assistant included. Planned (and confirmed with the
user) as three phases; full plan text lives in
`C:\Users\lipeng\.claude\plans\goofy-wiggling-spark.md` on this machine.
Two architecture corrections came out of researching the plan, worth
recording since they refine the user's own stated hypothesis:

- **SSH stays its own tool repo** -- `roc_desk-ssh` is not moved into
  `roc_desk_common`. It already ships a one-call `RocDeskSshAppState::new(db_path, app_handle)`
  constructor (connection manager/pools/known-hosts/credentials/transfer
  log all included) and 65 already-verified commands; moving it into
  `common` would break the "the core layer depends on no tool repo" rule.
  `roc_desk-workspace` instead takes a **direct tool-to-tool dependency**
  on `roc_desk-ssh` (same pattern as the existing `workspace -> editor`
  dependency for `<EditorPane/>`).
- **The real "move to common" candidate turned out to be `ChangeStore`/
  `diff`/`CodingTarget`**, not SSH or more AI plumbing (`roc_desk_common::ai`/
  `agent_llm`/`agent_confirm`/`agent_todo` already cover that from earlier
  phases). Host's own `commands/sql.rs` (the SQL AI assist panel) reuses
  the exact same `crate::coding::ChangeStore` type the AI coding agent
  uses for Diff/Accept/Undo -- i.e. host itself already treats this as a
  shared primitive, just not one either tool repo could depend on (SQL
  depending on the workspace tool would be backwards).

### Phase 1 — done, verified, pushed

Moved `ChangeStore`/`FileChange`/`FileSyncInfo`/`ChangeStatus`/`DiffLine`/
`generate_diff`/`CodingTarget` from `roc_desk-workspace`'s `coding::{changes,diff,target}`
into a new `roc_desk_common::change_store` module (`common-v0.13.0`).

- **Decoupling, not a straight copy**: `ChangeStore::stage`/`accept` used
  to hard-code `&roc_desk_ssh::ssh::SshConnectionPool`/
  `&roc_desk_ssh::agent::AgentConnectionPool` parameters (for the "Accept
  on a target that auto-commits to git" path) -- `roc_desk_common` must
  never depend on a tool crate, so that became a small
  `#[async_trait] pub trait GitCommitter { async fn commit_file(...); }`.
  `roc_desk-workspace` supplies the real implementation
  (`coding::git_ops::SshGitCommitter`, wired in via `ChangeStore::with_committer`
  in `coding::commands::build_new_session`, only when `state.ssh` is
  `Some`); a purely local-only caller like the future SQL port never
  constructs one.
- **Also decoupled from `tauri::AppHandle` entirely** (not originally
  planned, found while trying to unit-test the moved code): `stage`/
  `accept` used to call `app_handle.emit("coding:git-commit-result", ...)`
  internally, which meant every call site needed a *real* `AppHandle` --
  including in a `roc_desk_common` unit test, where constructing one via
  `tauri::test::mock_app()` crashed the test binary at startup
  (`STATUS_ENTRYPOINT_NOT_FOUND`, looks like a native/DLL mismatch specific
  to `tauri`'s `test` feature in this environment, not investigated
  further since the real fix was better anyway). Changed `stage`/`accept`
  to return `Option<GitCommitOutcome { path, output }>` instead of emitting
  themselves; the caller (`coding::session::stage_change`, and
  `lib.rs`'s `coding_accept_change` command) emits the event. Net result:
  `roc_desk_common::change_store` has no `tauri` dependency at all, and
  gained two real unit tests (stage→pending, accept→undo round trip)
  that didn't exist before.
- Side benefit: `coding_accept_change` no longer requires `state.ssh` to
  be `Some` at all (it only needed the pools to pass into `ChangeStore::accept`,
  which no longer takes them) -- one fewer place the "local workspace
  blocked on SSH" bug from phase 7 shows up, ahead of phase 2 fixing the
  root cause.
- Verified: `roc_desk_common`'s own `cargo test` (19 passing, two new);
  `roc_desk-workspace`'s `cargo check`/`cargo test` (lib *and* standalone,
  25 passing); `grep -c 'name = "roc_desk_core"' Cargo.lock` confirms a
  single source after bumping `roc_desk-explorer`/`-editor`/`-ssh`/
  `-workspace` in dependency order (`common-v0.13.0` throughout).

### Phase 2 — done, verified, pushed (2026-10-08)

**改动的仓库**：`roc_desk-explorer`、`roc_desk-editor`、`roc_desk-ssh`、`roc_desk-workspace`。

- `roc_desk-workspace` standalone 现在自己构造一份 `roc_desk_ssh::RocDeskSshAppState`
  （独立的 `ssh.db`，和 `workspace.db` 分开），喂进 `WorkspaceAppState::with_ssh`——
  `workspace_open_remote` 从"永远返回功能未启用"变成真的能用，本地工作区也顺带被
  修好（`CodingSession` 的签名本来就无条件需要*某一份*连接池引用，不管 target 是不是
  本地，见 phase 7/阶段一笔记）。
- **调研中发现一个计划文本没覆盖到的缺口，另外问过用户是否要一并补齐**：`EditorPane`
  有"本地简单模式"（`workspaceId=null`，走 `local_*`）和"工作区模式"
  （`workspaceId` 非空，走 `fs_*`，本地/远程统一由 `WorkspaceHandle.file_ops` 分发）
  两种，`roc_desk-workspace` 之前只用了前者——意味着即使接上 SSH，人工在编辑器里也
  打不开/编辑不了远程文件（AI 编程助手不受影响，它走的是已经 target-aware 的
  `ChangeStore`/`FileOps`）。用户选择"一并补齐"，于是这轮额外做了：
  - `roc_desk-workspace/lib/src/lib.rs` 新增 `fs_*` 命令层（16 个：
    list_dir/read_file/write_file(_with_encoding)/read_file(_with_encoding)/
    supported_encodings/read_binary_preview/open_externally/
    convert_legacy_office_to_pdf/inspect_binary/peek_is_binary/inspect_jar/
    delete/rename/copy/create_dir），原样照搬宿主 `commands/fs.rs` 的逻辑，但
    读写一律走 `WorkspaceHandle.file_ops`（而不是宿主旧版的 `LocalFileOps` 硬编码），
    `guard_local_path` 只在 `WorkspaceKind::Local` 时做边界校验（远程交给 `FileOps`
    自己的实现，和宿主旧版注释的取舍一致）。`fs_open_externally` 复用
    `roc_desk-explorer` 的 `open_path_or_launch_exe`（为此把该函数从 `roc_desk-explorer`
    的私有 fn 改成 `pub fn`，标 `v0.2.12`，这是本阶段唯一一次改动一个"已完工"工具仓库
    的代码）。
  - `roc_desk-editor` 的 `src-web` barrel（`src/index.ts`）补充导出
    `useFileTreeOperations`/`parentOf`/`baseName`/`flattenVisible`/`FileTreeBackend`/
    `fsService`——这些在该仓库内部本来就已经是和宿主 `ExplorerTree.tsx`/
    `useFileTreeOperations.ts` 逐字节一致的实现（之前只是没通过 barrel 导出），直接
    复用，不新写一份。`editorStore.openPreview(workspaceId, path)` 本来就已经在
    `workspaceId` 非空时走 `fsService`/`fs_*`——这意味着只要后端 `fs_*` 命令存在，
    `EditorPane` 切到工作区模式后不需要改一行自己的代码就能支持远程文件。标 `v0.2.12`。
  - 新增 `roc_desk-workspace/src-web/src/components/Workspace/ExplorerTree.tsx` +
    `stores/explorerStore.ts`：原样搬自宿主 `Explorer/ExplorerTree.tsx`/
    `stores/explorerStore.ts` 的懒加载/多选/剪切复制粘贴/重命名/新建/删除/拖拽移动/
    右键菜单逻辑，`fsService`/`useFileTreeOperations` 都从 `@roc_desk/tool-editor`
    导入，不重复实现。故意去掉两个这个独立版没有对应基础设施的功能："运行脚本"
    （需要宿主那套多标签终端 session store，这个工具的终端面板是单一简单终端）和
    "导入到本地搜索引擎"（host 独有的日志搜索模块，完全没有对应物）。
  - `App.tsx`：`EditorPane`/侧边栏从"本地简单模式 + `LocalFileTree`"换成"工作区模式
    + `ExplorerTree`"，本地和远程workspace统一走这一套；新增"连接远程主机"入口
    （`RemoteWorkspaceDialog`）和"打开文件夹"并列；远程工作区隐藏"终端"/"Git" 两个
    底部标签（这两个本来就是本地专属，宿主自己的编程工作区screen 也没给远程开这两个，
    phase 7 已确认过，不是这轮引入的新限制）。
  - `roc_desk-ssh` 的 `src-web` 补了一个最小 barrel（`package.json` 改名
    `@roc_desk/tool-ssh`，新增 `src/index.ts`），只导出"连接远程主机并选择目录"这条
    流程真正用到的东西：`ConnectionForm`、`connectionService`/`connectionGroupService`/
    `sftpService`/`agentService`，以及**调研时才发现必须一起导出**的
    `HostKeyPromptHost`/`AgentCertPromptHost`/`register(HostKey|AgentCert)PromptListener`
    ——没有这两个全局事件监听/弹窗，第一次连接一台新主机触发的指纹 TOFU 确认会永远
    没人响应，`ssh_pool.get_or_connect` 相当于挂死。不导出该工具的其余组件（终端/RDP/
    SFTP 双栏浏览器/传输日志）。
  - `roc_desk-workspace/src-web`：新增 `RemoteWorkspaceDialog.tsx`（三步流程，原样
    搬自宿主同名组件，服务层换成 `@roc_desk/tool-ssh` 导出的那几个）+
    `Workspace/PasswordPromptDialog.tsx`（补录密码/配对令牌的小弹窗，直接搬，只依赖
    本仓库已有的 `ConfirmDialog`）+ `utils/windowsPath.ts`（Agent 目标的虚拟盘符根
    路径工具）。`App.tsx` 顶层挂载两个 TOFU 弹窗宿主 + 注册两个监听器。
  - `standalone/Cargo.toml` 新增对 `roc_desk_ssh` 的直接依赖（`v0.3.8`，和 `lib` 一致）
    ——这是又一次确认"每个有自己独立依赖声明的 crate 都要单独检查"教训的地方：
    `standalone/main.rs` 里要直接写 `roc_desk_ssh::RocDeskSshAppState::new(...)`，
    不经过 `roc_desk_workspace` 的任何转发，所以需要自己的直接依赖声明。
  - `standalone/main.rs` 只注册 `roc_desk_ssh::cmd::*` 里"打开远程工作区"这条流程
    真正用到的 14 个命令（连接/分组 CRUD、`sftp_list_dir`、
    `agent_test_connection`/`list_dir`/`list_roots`、`ssh_confirm_host_key`、
    `agent_confirm_cert`），不是全部 65 个——终端/RDP/上传下载这些不是这个流程要用的，
    宿主自己的编程工作区 screen 也从来没有过。
- 依赖链 bump（严格顺序，每步都单独 `cargo check` 验证）：`roc_desk-explorer`
  （`v0.2.12`）→ `roc_desk-editor`（`v0.2.12`，bump explorer 引用）→
  `roc_desk-workspace`（`lib`/`standalone` 都 bump editor/explorer 到 `v0.2.12`，
  新增 `base64`/`roc_desk_ssh` 依赖）。
- 验证：`roc_desk-workspace` 的 `cargo check`（lib + standalone）、`cargo test`
  （lib，25 passing，不受影响）、`grep -c 'name = "roc_desk_core"' Cargo.lock`
  （单一来源）；前端 `npx tsc --noEmit`（通过）+ `npx vite build`（通过，只有既有的
  Monaco chunk-size 警告）。未做端到端真机联调（新建 SSH 连接→打开远程工作区→
  在编辑器里改一个远程文件→AI 编程助手发消息）——下次有真实远程主机可测时补上。

### Phase 3 — done, verified, pushed (2026-10-08)

**改动的仓库**：`roc_desk-sql`。

- `roc_desk-sql` 的 `roc_desk_common`/`roc_desk_core` 依赖之前停在
  `common-v0.10.0`，比依赖链其它仓库落后了三个大版本——这是真实的版本漂移
  风险（不是理论上的，这次要用的 `roc_desk_common::change_store` 根本不存在
  于 v0.10.0），一并 bump 到 `common-v0.13.0`（`lib`/`standalone` 两处都要改，
  老教训）。
- `SqlAppState` 新增：`ai_provider_manager`（和 `roc_desk-workspace` 同一份
  模式）、`sql_ai_assistant`、`sql_agent_sessions`/`sql_agent_cancel_tokens`/
  `sql_agent_confirms`/`sql_agent_questions`/`sql_agent_history`（新
  `sql::agent::history::SqlAgentHistoryRepo`，原样搬自宿主
  `db/repo/sql_agent_history_repo.rs`）、`sql_changes`（阶段一搬进
  `roc_desk_common` 的那个 `ChangeStore`，SQL 侧永远 `CodingTarget::Local` +
  不设 `GitCommitter`——SQL 标签页是本地缓存文件，这个工具完全不需要像
  `roc_desk-workspace` 那样依赖 SSH）。
- 新命令：`ai_provider_*`（5 个，和 `roc_desk-workspace` 那组一样的理由，
  宿主这组命令在 `commands/ai.rs`，不随 SQL Agent 代码带过来）、
  `sql_agent_*`（start/new_session/close/set_provider/send_message/
  cancel_turn/resolve_confirm/answer_question/history_*，13 个，原样搬自
  宿主 `commands/sql_agent.rs`，只是把"同时拿 `State<AppState>` 和
  `State<SqlAppState>`"简化成只拿 `State<SqlAppState>`——这个工具没有宿主
  那个独立的 `AppState`）、`sql_ai_*` + `sql_accept/reject/undo/
  revert_change`（8 个，AI 生成/解释/优化/修复 SQL 面板，`stage`/`accept`
  换成了阶段一之后的新签名，不再需要传 SSH pool/`AppHandle`）。
- **调研中发现一个和"移植"预期不同的真实情况**：`sql_ai_*`（生成/解释/
  优化/修复面板）这套命令宿主自己的前端从来没有接过任何界面调用——
  `grep` 遍历整个宿主 `src-web` 没有一处引用 `sql_ai_generate` 等命令，
  这是宿主自己现状，不是这次漏做。按"独立版功能和 roc_desk.exe 完全
  一致"的目标，这意味着**不需要给这几个命令补前端界面**——补了反而是
  "比宿主自己做得更多"，偏离了对齐的目标。`sql_agent_*`（多轮对话面板）
  则确认是宿主真实在用的功能（`SqlDesk/SqlAgentPanel.tsx` 挂在
  `SqlWorkspace.tsx` 里），这条路径完整移植了前端。
- 前端新增 `components/SqlAgent/*`（`SqlAgentPanel`/`AgentMarkdown`/
  `ThinkingBlock`/`ToolCallProgress`/`QuestionDialog`/
  `SqlAgentHistoryDialog`/`SqlAgentConfirmDialog`/`ProviderManagerDialog`）+
  `stores/{aiProviderStore,sqlAgentStore}.ts` + `services/
  {aiProviderService,sqlAgentService}.ts` + `utils/{markdown,shellHighlight,
  formatTokens}.ts`，风格上延续"简洁实用"基调（这个工具自己维护一份，不
  是跨工具共享）。附件输入去掉了宿主版本里"Tauri 原生拖拽本地文件路径"
  这条路径（需要 `local_read_binary_preview` 这类命令，这个独立版的
  standalone 没有注册本地文件浏览命令，不值得单独为这一个输入方式去接）
  ，保留浏览器原生"选择文件"/粘贴这两种。`SqlWorkspace.tsx` 从两栏布局
  恢复成宿主的三栏布局（对象树 | 编辑器 | AI 工具，默认收起）。
- 验证：`roc_desk-sql` 的 `cargo check`（lib + standalone）、`cargo test`
  （lib，3 个既有测试套件全部通过，不受影响）、`grep -c 'name =
  "roc_desk_core"' Cargo.lock`（单一来源）；前端 `npx tsc --noEmit`
  （通过）+ `npx vite build`（通过，只有既有的 Monaco chunk-size 警告）。
  未做端到端真机联调（配置 Provider → 连接数据源 → AI 助手发消息）——
  下次有真实数据库可测时补上。

至此，approved 的三阶段计划（`roc_desk_common::change_store` 抽象 → SSH 接入
`roc_desk-workspace` → SQL Agent 接入 `roc_desk-sql`）全部完成。独立版
`roc_desk-workspace.exe`/`roc_desk-sql.exe` 和 `roc_desk.exe` 对应模块的
功能差距，到这里已经收敛到"宿主自己都没有的东西"（宿主自己没有的 `sql_ai_*`
界面、`fs_search_stream`/`fs_replace` 这个搜索面板，见阶段二笔记）为止。

### Host 依赖追赶（2026-10-09）

三阶段计划完成后，用户在独立版工具的实测中又修了一批问题（AI 编程助手面板
挪到右侧停靠栏、编辑器 Tab 样式、AI Provider 配置共享 `roc_desk.db`、远程
工作区补终端标签页、`.rock_desk` 目录路径对齐、SQL 新增"导出为 SQL"格式、
全局细滚动条 CSS），这些修复只落到了各工具仓库的新 tag，host 的
`src-tauri/Cargo.toml` 还停在旧 tag，没跟上。本轮把 host 依赖追到各仓库
当时的最新 tag：

- `roc_desk-common`：`common-v0.10.0` → `common-v0.13.0`。
- `roc_desk-explorer`：`v0.2.8` → `v0.2.12`。
- `roc_desk-editor`：`v0.2.8` → `v0.2.12`。
- `roc_desk-ssh`（+ `roc_desk_protocol`）：`v0.3.5` → `v0.3.9`。
- `roc_desk-workspace`：`v0.2.7` → `v0.3.7`（见下，`v0.3.6` 还不够）。
- `roc_desk-sql`：`v0.3.5` → `v0.3.8`。

**升级过程中踩中两次版本漂移，两次都通过"在上游工具仓库里补发一个只改
依赖版本号的小版本"解决，而不是在 host 这边硬凑**：

1. `roc_desk-http` 的最新 tag `v0.2.5` 本身还钉在 `common-v0.10.0`——这个
   仓库没被三阶段计划碰过，没跟着一起升级过。host 的 `roc_desk_core`/
   `roc_desk_common` 升到 v0.13.0 后，`roc_desk_http::AppError` 和
   host 代码里 `?` 转换目标的 `roc_desk_core::error::AppError` 变成两份不同
   来源的类型，直接编译失败（42 个 E0277/E0308）。处理方式：在
   `roc_desk-http` 仓库里**只改** `lib/Cargo.toml`/`standalone/Cargo.toml`
   里 `common-v0.10.0` → `common-v0.13.0` 这两行，单独 commit（这个仓库当时
   还有一堆和此事无关的、未提交的"迁移成独立工具"在途改动，没有一起带上），
   打 tag `v0.2.6` 推送，host 再指向这个新 tag。
2. 同理，`roc_desk-workspace` 的最新 tag `v0.3.6` 自己内部钉的
   `roc_desk_ssh`/`roc_desk_protocol` 还是 `v0.3.8`，而 host 直接依赖的是
   刚升到的 `v0.3.9`——`Cargo.lock` 里一度同时出现两份
   `roc_desk_ssh`/`roc_desk_protocol`（`grep -c` 分别是 2 和 3，但没有触发
   编译错误，运气好在于 host 代码没有跨这两份类型做 `?`
   转换）。同样在 `roc_desk-workspace` 仓库里把 `lib/Cargo.toml`/
   `standalone/Cargo.toml` 的 ssh 依赖改成 `v0.3.9`，验证 `cargo check
   --workspace` 通过后单独 commit、打 tag `v0.3.7` 推送，host 再指向
   `v0.3.7`。
   验证收尾时 `grep -c 'name = "roc_desk_core"'`/`roc_desk_common`/
   `roc_desk_ssh`/`roc_desk_protocol`/`roc_desk_explorer`/
   `roc_desk_editor`/`roc_desk_http`/`roc_desk_sql`/`roc_desk_workspace`
   全部回到 1（唯一例外是 host 自己 workspace 里本来就有的本地
   `protocol` crate，和 roc_desk-ssh 的 `roc_desk_protocol` 恰好同名但
   互不相干，这不是漂移）。

- API 破坏性变更：`roc_desk_workspace::WorkspaceAppState::new` 从
  `(db_path, cache_root)` 两个参数变成了
  `(db_path, ai_providers_db_path, workspace_db_path, cache_root)` 四个
  参数。host 这边调用的 `WorkspaceAppState`（见 `lib.rs`
  注释——host 命令层完全不用它，纯粹为了满足某些内部 trait bound 而初始化）
  三个 db 路径参数原样传同一个 `workspace_tool.db`，维持升级前的隔离行为
  不变。
- 验证：`cargo check`（host，逐步升级每步都跑了一次）、`cargo test`
  （host，57 个既有测试全部通过）、`.\build-portable.ps1`（前端 `vite
  build` + host release + `roc_desk_agent` nightly 三段全部成功，新
  `roc_desk.exe` 已拷进 `bin\` 和
  `F:\code\wuyou\roc_tools\roc_desk-releases\bundle\`）。
  中途遇到 F 盘写满（196G 用满只剩 40KB，和这次升级无关的历史编译缓存
  堆积），清理了 `roc_desk-common`/`explorer`/`editor`/`http`/`sql`/`ssh`/
  `workspace` 七个工具仓库的 `target` 目录（`cargo clean`，共释放约 40GB）
  后才能继续编译，记一笔供下次遇到同样情况时参考。
  未做端到端真机联调——只是依赖版本追赶，没有改动 host 自己的业务逻辑
  （除了上面那一处签名适配），风险主要在"编译通过 + 既有测试通过"这个
  覆盖范围内。

### SQL 导出格式补全 + 独立版 Windows 文件关联 + 编辑器界面对齐（2026-10-09）

同一天内在依赖追赶之后又做的三件小事，记在一起：

1. **host 的 SQL 导出面板补上"导出为 SQL（INSERT 语句）"格式**：
   `roc_desk_sql` crate 自 v0.3.8 起后端（`sql::transfer`）早就支持这个
   格式（断点续传），但 host 前端 `TransferDialog.tsx` 的格式下拉框一直
   没加这个 `<option>`，`types/bindings.ts` 里 `TransferFormat` 类型也还
   停在 `"csv" | "json"`——纯粹是跟 roc_desk-sql 这边前端分别维护一份导致
   的遗漏，两处都只差一行。
2. **roc_desk-editor（v0.2.13）/ roc_desk-sql（v0.3.9）两个独立版工具新增
   Windows 文件关联支持**（双击 .txt 默认用编辑器打开、.sql 默认用 SQL
   工作台打开）：各自的 `standalone/` 加 `tauri-plugin-single-instance`
   依赖，冷启动 argv 和已运行实例收到的二次启动都走同一套
   "`PendingOpenPaths`/`take_pending_open_paths` 命令 + `open-file-paths`
   事件"机制——和 host `roc_desk.exe` 自己的 `extract_open_paths`/
   `pending_open_paths` 机制是同一个模式，复用了这个思路而不是另起一套。
   - `roc_desk-editor` 直接调用已有的 `editorStore.openStandaloneFile`。
   - `roc_desk-sql` 的标签页模型是"内部实体+自己的草稿存储"，不是直接
     绑定磁盘路径，所以额外加了一个 `sql_read_external_file_text` 命令
     读盘，前端 `createTab`+`setTabContent` 灌内容；新建标签页需要已经
     选中数据源，没连上时先把路径排进一个 `pendingSqlPathsRef`，等
     `currentDataSourceId` 变化后的 `useEffect` 里再补开（弹一次 toast
     提示用户先选数据源）。
   - `roc_desk-releases/bundle/` 里的 `roc_desk-editor.exe`/`roc_desk-sql.exe`
     已更新到这两个新 tag 对应的构建。
   - `roc_desk-releases` 仓库新增 `register-file-associations.ps1`（随
     release zip 一起打包，`bundle.yml`/`publish-local.ps1` 都会把它拷进
     `bundle\`）：把 .txt/.log/.md/.json/.yaml/.yml/.ini/.conf/.cfg/.xml/
     .csv 关联到 `roc_desk-editor.exe`、.sql 关联到 `roc_desk-sql.exe`，
     只写 `HKCU\Software\Classes`（当前用户，不需要管理员权限），支持
     `-Unregister` 撤销。**真机验证发现一个 Windows 自身的限制**：
     Windows 8 开始，一个扩展名如果已经有"默认应用"（存在
     `UserChoice` 注册表项，带哈希签名防篡改），脚本没法静默覆盖——
     实测 `.sql`（这台机器上从没设置过默认应用）注册完立刻双击生效，
     但 `.txt`（默认已经是记事本）注册完双击仍然用记事本打开，尽管
     `HKCU\Software\Classes\.txt` 的默认值已经确实改成了
     `RocDesk.txt`、右键"打开方式"列表里也正确出现了
     roc_desk-editor.exe——这类已经有默认值的扩展名需要用户自己手动
     右键"打开方式"选一次+勾选"始终使用此应用"，没有纯注册表脚本能绕过
     这层保护。脚本运行完会打印这条提示，README 里也写了。
3. **roc_desk-editor 独立版的界面式样和 host 对齐**（v0.2.13 → v0.2.14，
   两版）：用户反馈这个独立 exe 和 host 的 `roc_desk.exe`
   （`mode === "editor"` 独立编辑器模块窗口）差距明显。v0.2.13 把顶部
   工具条换成和 host 同一套 markup/CSS class（`Code2` 图标 + "本地文件"
   标题 + "打开文件 (Ctrl+O)" 按钮 + 主题切换，"打开文件"按钮/主题切换
   用 `justify-content: space-between` 推到最右，照抄 host
   `components.css` 的布局思路），侧栏从固定 260px 改成可拖拽宽度+
   `localStorage` 持久化。host 那个模式窗口还有的"返回首页"按钮和底部
   可折叠本地终端面板没有带——前者在独立 exe 里没有意义（没有宿主那个
   多模块首页/启动器概念），后者需要整套本地 PTY 终端支持（`portable-pty`
   + 一堆 `pty_*` 命令），这个工具目前完全没有这部分后端，是比"界面
   式样"大得多的另一个功能，没有一并加上。
   - **v0.2.13 验证时踩了一个坑，之后发现这坑本身就是真 bug**：用 Win32
     `PrintWindow` 截图看不到右上角的"打开文件"/主题切换图标，一度怀疑
     是截图方法本身的问题——用 `getBoundingClientRect()`/
     `getComputedStyle()` 直接读 DOM 显示元素"渲染正常"（`display:flex`、
     `visibility:visible`、`opacity:1`、尺寸非零），于是当时错误地下结论
     "纯粹是截图工具在这个区域失效，和代码无关"就把 v0.2.13 发布出去了。
     **后来用户在真机上用普通方式截图反馈"UI还是不对头"，同样的图标还是
     看不见**——换成更可靠的截图方式（`SetWindowPos` 置顶 +
     `Graphics.CopyFromScreen` 直接拷屏，不是 `PrintWindow`）复现后确认
     这两个图标在真实画面上确实不可见，DOM 测出来"正常"只是 DOM 测的
     东西和实际画面是两回事。进一步用 CSS `order` 属性做了个对照实验
     （保持 DOM 顺序不变，只用 `order:-1` 把 `.quick-tools` 视觉上挪到
     最左边）：挪到左边之后两个图标就能看见了，而原来在左边的"本地文件"
     标题被视觉挪到最右边之后反而看不见了——说明问题跟 DOM 顺序无关，
     跟"视觉上处于这一行最右侧"这个位置本身有关（这个窗口顶部偏右的
     一小块区域，大致 y<30px、x 较大的部分，不管用 flex auto margin、
     `justify-content:space-between` 还是 `order` 属性把内容摆过去，
     结果都是"摆过去的内容在真实画面上画不出来"，换成
     `position:fixed` 固定坐标到同一块区域现象也一样，对窗口做一次
     resize 强制重绘也没用）。根因没有彻底查清楚（怀疑和这台机器上
     WebView2/合成器在这块区域的某种显示管线问题有关，可能是这个特定
     运行环境的问题，不确定是否是 WebView2 通用 bug），v0.2.14
     的做法是**绕开**而不是修复：顶部工具条改成"打开文件"/主题切换
     按钮都紧跟在"本地文件"标题后面、不推到最右，整行左对齐、不出现
     在这块有问题的区域。**教训**：`getBoundingClientRect`/
     `getComputedStyle` 证明的是 DOM/布局计算层面"应该"渲染成什么样，
     不能替代"实际画面上真的看得见"这个最终判断；只有自动化截图方法
     可疑时才换一种截图方法交叉验证，不能仅凭 DOM 测量就反过来断言
     "所有截图方法都有问题、代码本身没问题"——这次这个论断在 v0.2.13
     发布前就是错的，需要用户在真机上复现才被纠正过来。
