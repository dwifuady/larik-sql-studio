# ADR 0003: Credential Storage, Secret Hygiene, Win32 FFI & Legacy Tauri Migration

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-core/src/secrets.rs`, `crates/larik-core/src/store.rs`, `crates/larik-core/src/migration.rs`, `crates/larik-core/src/snippets.rs`, `crates/larik-core/src/run_log.rs`, `crates/larik-core/src/mcp.rs`, `crates/larik-data/src/lib.rs`, `crates/larik-provider-mongodb/src/lib.rs`

## Context

Larik SQL Studio stores workspace state in local SQLite while keeping credentials in Windows Credential Manager, and automatically upgrades installations from the legacy Tauri version without losing saved connections, secrets, history, or notes.

## Decisions

### 1. Secret Hygiene & Separation from SQLite Snapshots
- Provider-declared password fields are **always** stripped from `ConnectionProfile` before serializing `studio_snapshot` JSON into SQLite—even when `sync_secrets` is `false`.
- For MongoDB, **both** `connectionString` and `password` must be declared as password/secret keys because `mongodb://` / `mongodb+srv://` URIs can embed credentials. UI display paths parse URIs solely through `MongoUriSummary` / `summarize_mongo_uri`, which never returns passwords.
- `StudioStore::save_snapshot_only` persists sanitized UI state (tab switches, editor edits, layout toggles) without touching Windows Credential Manager, avoiding slow Win32 IPC on high-frequency UI mutations.
- Windows Credential Manager target keys follow the stable persisted contract `Larik.SqlStudio/space/{space_id}/{key}`; changing this format would orphan existing user credentials.
- In `larik-mcp`, passwords are never retained inside `McpSpaceRecord` and are hydrated via `StudioStore::hydrate_mcp_profile` only immediately before opening a live connection.

### 2. Win32 Credential Manager FFI Safety Invariants (`crates/larik-core/src/secrets.rs`)
The `unsafe` blocks calling `CredReadW`, `CredWriteW`, `CredDeleteW`, and `CredFree` uphold these invariants:
- `CredReadW`: `target_w` is null-terminated UTF-16; `cred_ptr` receives a Windows-allocated `CREDENTIALW` pointer that is dereferenced only while valid and immediately copied before being freed unconditionally with `CredFree`.
- `CredWriteW`: All stack-owned buffers (`target_w`, `user_w`, `blob`) whose raw pointers are placed into `CREDENTIALW` remain alive for the entire duration of the `CredWriteW` call.
- `CredDeleteW`: `target_w` is null-terminated UTF-16.

### 3. Automatic, Idempotent Legacy Tauri Migration & Rollback Safety
- Legacy Tauri database discovery (`%APPDATA%\larik\larik-sql-studio\data\larik.db`) runs **only** when using the default app data directory—never when `LARIK_STATE_DIR` is explicitly set (protecting dev and test environments from importing user state).
- Before migrating in place, a self-contained single-file backup is created using SQLite `VACUUM INTO` (capturing uncommitted WAL frames without mutating the source DB), falling back to file copy only if `VACUUM INTO` fails.
- Incompatible legacy `mcp_log` / `mcp_approvals` tables are renamed rather than dropped before `ensure_schema()` creates the current tables.
- Legacy Tauri built-in snippets used random UUIDs; migration removes those rows so current built-ins can be seeded idempotently under deterministic `builtin:{scope}:{trigger}` primary keys via `INSERT OR IGNORE` on every launch without creating duplicates or overwriting user edits.
- Legacy sticky notes referencing archived tabs are mapped by **both** `original_tab_id` and `archived_tabs.id` so notes link cleanly across the historical foreign-key ambiguity.
- Legacy `keyring` v3 (`windows-native`) credentials live under target `{user}.{service}` (`space-password/{space_id}.larik-sql-studio`) with **UTF-16LE** blobs (falling back to UTF-8 if invalid UTF-16LE). Migration copies passwords into the new target namespace and intentionally leaves old Tauri credentials untouched alongside the DB backup to preserve rollback safety.

### 4. Timestamp & Persistence Contracts
- `run_log.recorded_at` and `mcp_log.recorded_at` use SQLite UTC `datetime('now')` format (`YYYY-MM-DD HH:MM:SS`), distinct from RFC3339 strings used in JSON snapshots and notes.
- `ArchivedTabState::age_label` retains backward compatibility with pre-RFC3339 `"HH:MM"` strings saved by older snapshots by returning the raw string if RFC3339 parsing fails.
- `StudioStore::record_run` (`run_log`) is best-effort by contract so local history persistence failures never discard interactive query results.
