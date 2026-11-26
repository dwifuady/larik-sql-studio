# Architecture Decision Records

This directory captures non-obvious architectural, security, compatibility, performance, and UX decisions for Larik SQL Studio.

Code comments are banned by the workspace comment-ratchet lint (`cargo test -p larik-core --test comment_lint`). Any non-obvious decision or invariant belongs here rather than in inline source comments.

## Index

| ADR | Title | Scope |
| --- | --- | --- |
| [0001](0001-provider-plugin-isolation-and-session-topology.md) | Provider Plugin Isolation & Per-Tab Session Topology | `larik-data`, `larik-core`, `larik-ui` |
| [0002](0002-mcp-security-and-read-only-enforcement.md) | MCP Fail-Closed Security, Approval Lifecycles & Read-Only Enforcement | `larik-core`, `larik-data`, `larik-mcp`, `larik-provider-mongodb`, `larik-ui` |
| [0003](0003-credential-storage-secret-hygiene-and-tauri-migration.md) | Credential Storage, Secret Hygiene, Win32 FFI & Legacy Tauri Migration | `larik-core`, `larik-data`, `larik-provider-mongodb` |
| [0004](0004-database-provider-semantics-and-transport-security.md) | Database Provider Semantics & Transport Security (SQL Server, PostgreSQL, MongoDB) | `larik-provider-mssql`, `larik-provider-postgres`, `larik-provider-mongodb` |
| [0005](0005-sql-language-autocomplete-and-snippet-expansion.md) | Cross-Dialect SQL Lexing, Autocomplete Ranking & Snippet Expansion | `larik-data` |
| [0006](0006-results-grid-editing-and-preview-performance.md) | Results Grid Virtualization, Staged Row Editing & Cell Preview Performance | `larik-data`, `larik-ui` |
| [0007](0007-windows-gpui-shell-mica-and-workspace-ux.md) | Windows 11 GPUI Shell, DWM Mica, Title Bar Hit-Testing & Workspace UX | `larik-core`, `larik-ui` |
