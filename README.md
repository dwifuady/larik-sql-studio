# Larik SQL Studio

A fast, native SQL studio built in pure Rust with [GPUI](https://www.gpui.rs/) (`gpui-kit`).

Larik organizes database connections into Arc-inspired **Spaces** (workspaces) with pinned tabs, collapsible folders, a tree-sitter SQL editor, virtualized multi-result-set grids, statement-anchored notes, query history, and a built-in **Model Context Protocol (MCP)** server for AI coding agents.

## Features

- **Spaces & Tab Organization**
  - Color-coded workspaces, each bound to its own database connection profile and active database.
  - Pinned tabs, collapsible tab groups/folders, drag-and-drop ordering, and automatic inactivity archiving with full restore.
- **SQL Editor & Productivity**
  - Tree-sitter SQL syntax highlighting, statement-under-caret execution (`Ctrl+Enter`) or full script execution (`F5`), and multi-statement execution.
  - Built-in and custom dialect-aware SQL snippets with tab-stop placeholders and autocomplete.
  - Schema explorer pane for browsing databases, tables, views, columns, and routines.
- **Results & Export**
  - Virtualized result grids built to stay responsive on large result sets, supporting multiple result sets and server `PRINT`/message output per run.
  - Structured query run log (`run_log`) with search and configurable retention (30 / 90 / 120 days).
- **Statement-Anchored & Tab Notes**
  - Attach `Note`, `TODO`, `Decision`, or `Observation` annotations to specific SQL statements or as tab-level scratchpads.
  - Fingerprint anchoring automatically tracks and rebases notes as you edit or reformat queries, and preserves notes when tabs are archived.
- **Built-in Database Providers (SQL Server, PostgreSQL, MongoDB + Demo)**
  - Native SQL Server (`mssql`) connectivity via `tiberius` (TLS, `trustServerCertificate`, Windows auth / SQL auth).
  - Native PostgreSQL (`postgres`) connectivity via `tokio-postgres` + `postgres-native-tls` (per-database sessions, SSL modes `Disable` through `VerifyFull`, notices, catalog schema & routines).
  - Native MongoDB (`mongodb`) connectivity via the official `mongodb` driver supporting both T-SQL-on-MongoDB (`SELECT`, `JOIN`, `GROUP BY`, `INSERT`, `UPDATE`, `DELETE`, `USE`) and `db.<collection>.<method>(...)` mongosh passthrough, plus schema sampling and UUID/ObjectId coercion.
  - Passwords and MongoDB connection strings are stored exclusively in Windows Credential Manager and never written to SQLite.
  - Built-in offline Demo provider for instant exploration and testing.
- **AI Agents via MCP (`larik-mcp`)**
  - Local stdio MCP server exposing schema inspection, read/write query execution, and `open_in_studio`.
  - Fail-closed security model: global opt-in toggle, per-workspace access levels (`Off`, `Read`, `Write`), audit log, and interactive in-app approval prompts over a local named pipe before any write statement runs.

## Tech Stack & Workspace Structure

Larik SQL Studio is a pure-Rust Cargo workspace (Rust 2024 edition) rendered directly on the GPU with `gpui-kit`:

| Crate | Purpose |
| --- | --- |
| `crates/larik-data` | Provider traits (`DatabaseProvider`, `DatabaseConnection`), connection profiles, schema types, SQL language definitions, and snippet primitives. |
| `crates/larik-provider-mssql` | Native Microsoft SQL Server provider built on `tiberius` + `tokio`. |
| `crates/larik-provider-postgres` | Native PostgreSQL provider built on `tokio-postgres` + `postgres-native-tls`. |
| `crates/larik-provider-mongodb` | Native MongoDB provider built on `mongodb` with T-SQL compiler, mongosh passthrough, and fail-closed read-only policy. |
| `crates/larik-provider-demo` | In-memory demo database provider for local development and testing. |
| `crates/larik-core` | Core studio state (`StudioState`, `SpaceState`, `TabState`), SQLite persistence (`StudioStore`), Windows Credential Manager secret backend, notes anchoring engine, snippets, run log, MCP types, and legacy Tauri DB migration. |
| `crates/larik-mcp` | Standalone stdio MCP server binary (`larik-mcp.exe`) with named-pipe approval bridge to the desktop app. |
| `crates/larik-ui` | Native GPUI desktop application binary (`larik-sql-studio.exe`). |

## Prerequisites

- **Rust** (stable toolchain supporting Rust 2024 edition)
- **Windows**: Visual Studio Build Tools (MSVC) and Windows SDK (for `rc.exe` icon embedding and native GPUI DirectX/Win32 backend)

## Building & Running

```powershell
# Run the desktop app (debug)
cargo run --bin larik-sql-studio

# Build all binaries (larik-sql-studio.exe and larik-mcp.exe)
cargo build --workspace

# Build optimized release binaries
cargo build --workspace --release

# Run the full workspace test suite
cargo test --workspace
```

### State Directory

By default, Larik SQL Studio stores its SQLite state database at:

```text
%LOCALAPPDATA%\Larik SQL Studio\studio.db
```

Set the `LARIK_STATE_DIR` environment variable to isolate state into a custom directory (useful for development or testing):

```powershell
$env:LARIK_STATE_DIR = "C:\temp\larik-dev"
cargo run --bin larik-sql-studio
```

## MCP Server Setup

Build the workspace to produce `larik-mcp.exe` alongside `larik-sql-studio.exe`, then enable **AI Agents (MCP)** inside the app and set per-workspace access levels (`Read` or `Write`).

The in-app **MCP** page provides copy-paste setup commands for Claude Code, Cursor, VS Code, and Codex. For example, with Claude Code:

```powershell
claude mcp add larik -- "C:\path\to\larik-mcp.exe"
```

Optional flags for `larik-mcp`:
- `--state <dir>`: Override the studio state directory (equivalent to `LARIK_STATE_DIR`).
- `--pipe <name>`: Override the local approval named pipe path.

## Upgrading from Legacy Tauri Larik SQL Studio

On first launch, Larik SQL Studio automatically detects an existing Tauri-era Larik SQL Studio database—either in the legacy Tauri app data directory (`%APPDATA%\larik\larik-sql-studio\data\larik.db`) or in-place—and performs a one-time, idempotent migration:

1. **Automatic backup first**: A self-contained backup copy (`larik.tauri-backup.db`) is created next to `studio.db` before any changes are written, and the original legacy database and Windows Credential Manager entries are left untouched.
2. **Full data migration**: Migrates workspaces/connections, saved passwords (decoding legacy `keyring` UTF-16LE Windows credentials and SQLite fallback passwords into the new credential namespace), tab folders, open query tabs, archived tabs, custom snippets and built-in toggle states, sticky notes (anchored to their SQL lines), app preferences, and MCP activity/approval logs.
3. **Safe & idempotent**: A `tauri_migration_v1` marker is recorded in `studio_meta` so migration runs only once and never overwrites an already-initialized GPUI studio database.

## Releasing

Workspace version is managed in a single place: `[workspace.package].version` in `Cargo.toml` (inherited by all crates, embedded in Windows `VERSIONINFO`, and validated by the release workflow).

To publish a new release after merging to `main`:

```powershell
# 1. Ensure Cargo.toml [workspace.package].version matches the release (e.g. 0.11.0)
# 2. Tag and push
git tag v0.11.0
git push origin v0.11.0
```

Pushing a `v*` tag triggers `.github/workflows/release.yml` on `windows-latest`, which runs tests, builds release binaries, packages an Inno Setup Windows installer, portable ZIP (`larik-sql-studio.exe` + `larik-mcp.exe`), standalone portable `.exe`, and `SHA256SUMS.txt`, and uploads them to a **draft GitHub Release** for review.

## License

MIT — see [LICENSE](LICENSE).
