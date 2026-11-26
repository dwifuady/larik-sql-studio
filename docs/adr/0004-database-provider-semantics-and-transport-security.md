# ADR 0004: Database Provider Semantics & Transport Security

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-provider-mssql/src/lib.rs`, `crates/larik-provider-postgres/src/lib.rs`, `crates/larik-provider-mongodb/src/*`

## Context

Each supported database engine has distinct transport security defaults, wire-protocol quirks, schema-introspection trade-offs, and query compilation semantics that cannot be inferred purely from method signatures.

## Decisions

### 1. SQL Server (`larik-provider-mssql`)
- **Unified TCP + LocalDB transport:** `MssqlStream` wraps both TCP streams and Windows LocalDB named pipes so a single `tiberius::Client` implementation handles both transports.
- **LocalDB IPC & auto-start:** Because `(localdb)\<instance>` communicates over local Windows named-pipe IPC, LocalDB connections configure `EncryptionLevel::Off` with `trust_cert()`. If the instance pipe is unavailable, the provider runs `sqllocaldb start <instance>` once before failing.
- **Schema metadata source:** Column introspection queries `sys.columns` (rather than `INFORMATION_SCHEMA.COLUMNS`) specifically so `is_identity` and `is_computed` flags are available for safe row-edit validation.
- **TDS `datetime` rounding:** Legacy TDS `datetime` values (days since `1900-01-01` plus `1/300`th-second fragments since midnight) are converted with SQL Server's standard millisecond rounding (`.000`, `.003`, `.007`).

### 2. PostgreSQL (`larik-provider-postgres`)
- **SSL modes & remote defaults:** `PostgresSslMode` follows `libpq` / `Npgsql` semantics. An empty `sslMode` defaults to `VerifyFull` for remote hosts; loopback hosts and Unix-socket paths never trigger transport warnings because traffic never crosses a network.
- **Host/port & legacy profile parsing:** `parse_host_and_port` falls back from `host` to the legacy `server` key for migrated profiles, and splits single-colon `host:port` only when the left-hand side contains no additional `:` so bare IPv6 literals like `::1` are never corrupted.
- **Multi-result `simple_query` flushing:** When `SimpleQueryMessage::RowDescription` arrives while a previous result set is still open without an intervening `CommandComplete`, the previous result set is flushed before starting the new one.

### 3. MongoDB (`larik-provider-mongodb`)
- **T-SQL to aggregation pipeline compilation:**
  - **Schema-sampled literal coercion:** Because collections are schemaless and SQL string literals do not match native BSON `ObjectId`, `Binary(Uuid)`, or `DateTime` values in `$match`, `MongoConnection` caches sampled field types per lowercased `(database, collection)` during `get_schema()` and passes them to `TSqlMongoCompiler` for automatic literal coercion.
  - **`HAVING` stage ordering:** The `$match` stage for `HAVING` runs **after** `$project` so both `SELECT` aliases and renamed aggregate outputs (`__agg_N`) are directly addressable as top-level fields.
  - **`NOT` compilation:** Negated predicates compile to `{ "$nor": [sub] }` rather than `$not` because `$not` is field-scoped whereas `$nor` accepts arbitrary compound filter documents.
  - **Index-friendly comparison fast path:** Simple `field <op> constant` and `constant <op> field` predicates compile to direct MongoDB filter operators (flipping the operator when the constant is on the left) instead of `$expr` so MongoDB can use collection indexes.
  - **1-based SQL `SUBSTRING`:** Subtracts `1` from the start argument to bridge SQL's 1-based indexing and MongoDB's 0-based `$substrCP`.
- **BSON display & shared constructors:**
  - `Binary` UUID subtype 4 decodes big-endian; legacy subtype 3 (`UuidOld`) decodes using `.NET` mixed little-endian byte order (`Uuid::from_bytes_le`).
  - Nested BSON documents and arrays render as plain human-readable JSON without Extended JSON `$oid` / `$date` / `$binary` wrappers.
  - BSON shell constructors (`ObjectId`, `ISODate`, `UUID`, `NumberDecimal`, etc.) are evaluated by one shared helper used by both the T-SQL compiler and the `mongosh` parser.
- **Connection compatibility & restricted Atlas roles:**
  - Discrete host parsing retains fallback to the legacy `server` key for migrated profiles.
  - When both a URI and discrete credential fields are configured, URI credentials take precedence and discrete `user`, `password`, and `authSource` only backfill omitted URI values.
  - During `open()`, if `ping` against the default database fails, the provider falls back to pinging `admin` (`DEFAULT_AUTH_SOURCE`).
  - Because shared/restricted MongoDB Atlas roles frequently lack `listDatabases` privilege, `list_databases()` intentionally falls back to returning `[default_database]` so users can still inspect and query their configured database.
