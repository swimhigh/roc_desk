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
