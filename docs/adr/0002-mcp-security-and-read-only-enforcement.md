# ADR 0002: MCP Fail-Closed Security, Approval Lifecycles & Read-Only Enforcement

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-core/src/mcp.rs`, `crates/larik-core/src/store.rs`, `crates/larik-data/src/read_only_sql.rs`, `crates/larik-provider-mongodb/src/read_only.rs`, `crates/larik-mcp/src/*`, `crates/larik-ui/src/mcp_bridge.rs`, `crates/larik-ui/src/studio.rs`

## Context

`larik-mcp` exposes saved database workspaces to external AI agents over Model Context Protocol (MCP) stdio JSON-RPC. Because AI agents can generate arbitrary queries—including destructive writes or obfuscated payloads—every policy boundary must fail closed and maintain an auditable execution trail.

## Decisions

### 1. Fail-Closed Access Control & Discovery
- Both the global AI-agent kill switch (`studio_meta['mcp_enabled']`) and per-workspace access mode (`McpAccess`) strictly fail closed (`false` / `McpAccess::Off`) on missing rows, corrupt values, or SQLite read errors.
- `StudioStore::list_mcp_spaces` returns only non-secret workspace profiles, never touches the OS Credential Manager, and never fabricates synthetic default/demo workspaces on an empty database (unlike `StudioStore::load`).
- Every refusal in `StudioContext::require_space` records a `denied` audit entry in `mcp_log`.

### 2. Two-Layer Read-Only Defense in Depth
- **Provider opt-in:** `DatabaseProvider::read_only_policy()` defaults to `None`, refusing all `McpAccess::Read` queries until a provider explicitly supplies a `ReadOnlyStatementPolicy`.
- **Layer 1 (Static fail-closed classifier):**
  - **SQL (`read_only_sql.rs`):** Every semicolon-separated statement unit must start with an allowed leading keyword and contain no denied keyword or denied prefix. Strings (`'...'`, `E'...'`, `$tag$...$tag$`), quoted/bracketed identifiers (`"..."`, `[...]`), `--` comments, and **nested** `/* ... */` block comments (required for PostgreSQL and T-SQL parity) are stripped and replaced with a **single space** so adjacent tokens cannot merge into a bypass. Any unterminated quote, bracket, dollar-quote, or block comment fails closed (`false`).
  - **MongoDB (`larik-provider-mongodb/src/read_only.rs`):** Compiled T-SQL `SELECT` pipelines are AST-checked for `$out` / `$merge`. Raw `db.<coll>.aggregate(...)` shell calls are refused outright in read-only mode because arbitrary JavaScript syntax (e.g., unicode-escaped keys like `"\u0024out"`) cannot be statically proven read-only. Shell argument scanning fails closed on any `$out`/`$merge` key (quoted or bare) or unterminated string literal.
- **Layer 2 (Engine-level enforcement):** `QueryExecutionOptions::read_only = Some(true)` requires providers to enforce read-only execution at the engine/transaction layer (for example `BEGIN TRANSACTION READ ONLY` or an unconditional transaction rollback), never relying solely on advisory client checks.

### 3. Isolated One-Shot Execution, Timeouts & Interactive Write Approval
- `StudioContext::with_connection` opens an isolated one-shot `DatabaseConnection` per tool call under a per-space concurrency semaphore, hydrates passwords from the OS secret backend just-in-time immediately before connecting, runs the operation, and closes the connection.
- `larik-mcp` enforces a hard 120-second per-query timeout ceiling regardless of workspace connection timeout settings.
- Interactive write approvals in the desktop studio auto-deny after **120 seconds** (`APPROVAL_TIMEOUT`), ticking once per second to keep the countdown fresh. The `larik-mcp` client enforces a **140-second** read backstop (`APPROVAL_TIMEOUT + 20s`) so the studio's own auto-deny takes precedence normally while preventing hangs if the desktop app exits mid-prompt.
- On studio startup, `sweep_mcp_approvals` marks any `pending` approval rows older than **10 minutes** as `orphaned`.
- `open_in_studio` (and Settings -> AI agents "Open as tab") always opens an agent's SQL in a new unpinned tab **without ever executing it**.

### 4. Transport & IPC Invariants
- **Stdio isolation:** In `larik-mcp`, **only** newline-delimited JSON-RPC 2.0 messages are written to `stdout`; all diagnostics go to `stderr`. On stdin EOF, the server drains all in-flight tool calls cleanly before exiting.
- **Named-pipe IPC:** Studio IPC uses Windows named pipes. On non-Windows targets, `run_pipe_loop` awaits `std::future::pending()` rather than returning immediately so the channel sender stays alive without triggering spurious closed-channel errors. In named-pipe integration tests, the first pipe server instance is created before spawning the server task so the client never races server creation.
