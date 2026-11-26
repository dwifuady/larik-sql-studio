# ADR 0001: Provider Plugin Isolation & Per-Tab Session Topology

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-data/src/lib.rs`, `crates/larik-core/src/session.rs`, `crates/larik-ui/src/studio.rs`, `crates/larik-ui/src/connection_dialog.rs`

## Context

Larik SQL Studio supports multiple heterogeneous database engines (SQL Server, PostgreSQL, MongoDB, and the offline Demo provider) from a single desktop shell and MCP server. Coupling UI dialogs, editor behavior, or session lifecycle to concrete driver crates makes adding providers expensive and risks leaking engine-specific state across tabs or workspaces.

## Decisions

### 1. Strict Provider Plugin Boundary
- The studio shell interacts with every database engine exclusively through [`DatabaseProvider`](../../crates/larik-data/src/lib.rs) and never depends directly on driver internals.
- Connection dialogs are rendered dynamically from provider-declared `ProviderSetting` descriptors (`provider_settings()`), keeping form layout provider-agnostic.
- Providers supply live transport/TLS warnings via `DatabaseProvider::transport_warning(&ConnectionProfile)` so the connection dialog can surface amber security warnings without knowing provider-specific TLS keys. Every provider text input in `ConnectionModal` subscribes to `InputEvent::Change` and triggers `cx.notify()` so transport warnings update live as fields are edited.
- Editor language mechanics (statement/batch splitting, identifier quoting, keywords, autocomplete) are owned by each provider behind the `EditorLanguage` trait seam.

### 2. Dedicated Control Connection + 1:1 Per-Tab Session Isolation
- `StudioSessionManager` owns a dedicated Tokio runtime and maintains:
  - **One control connection per workspace** for connection status, database listing, and schema introspection.
  - **One dedicated `DatabaseConnection` per query tab** rather than a shared connection pool.
- Giving each query tab its own connection preserves isolated SQL session state—temporary tables, `SET` options, active database context, and open transactions cannot bleed between tabs.

### 3. Startup Auto-Connect, Per-Tab Database Binding & Archive Survival
- Only the offline `demo` workspace auto-connects on first open for an instant "try it" experience. All real database workspaces remain disconnected until the user explicitly connects, avoiding surprise network connections or auth prompts on launch.
- Switching the active database applies **only** to the currently active query tab; sibling tabs retain their own bound database.
- Deleting a workspace intentionally leaves its archived tabs intact in History so users do not lose historical scripts. When restoring an archived tab whose original workspace was deleted, `StudioStore::reparent_tab_notes` updates `space_id` on all notes for that `tab_id` to the target workspace.
