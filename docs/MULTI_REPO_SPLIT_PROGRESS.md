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

## SSH/SFTP/RDP/Agent：tool-repo side done, host wiring deliberately deferred (2026-09-22)

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

## 编程工作区：tool-repo side partially done, host wiring deliberately deferred (2026-09-22)

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

## Known cross-tool dependency-graph quirk

- the host currently pulls
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
